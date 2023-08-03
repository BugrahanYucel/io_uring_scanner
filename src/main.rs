use std::io::{self, Error};
use std::os::fd::{AsRawFd, RawFd};
use std::env;
use std::rc::Rc;

use nix::sys::socket::{socket, AddressFamily, SockFlag, SockType, SockaddrLike, SockaddrIn};
// use nix::Error;
use libc::{self, iovec};

use io_uring::{squeue, opcode, types::Fd, IoUring, types::Timespec};
use io_uring::Probe;

use std::net::Ipv4Addr;
use ipnet::Ipv4Net;

use std::fs::File;
use std::io::{BufRead};
use std::str::FromStr;

// TODO: Parse arguments with flags using clap, looks good
// use clap::{Arg, App};

#[derive(Clone)]
struct SubnetInfo {
    ip_start : String,
    // ports : Vec<u16>,
    subnet_len : u8,
}

#[derive(Clone)]
pub struct EntryInfo {
    pub ip: Rc<SockaddrIn>,
    pub op_type: u8,
    pub fd: RawFd,
}

#[derive(Clone)]
pub struct EntryManager {
    entries : Vec<EntryInfo>,
}

impl EntryManager {
    pub fn new() -> Self {
        EntryManager { entries: Vec::new() }
    }

    pub fn add_entry(&mut self, ip: Rc<SockaddrIn>, op_type: u8, fd: RawFd) -> usize {
        let entry = EntryInfo { ip, op_type, fd };
        self.entries.push(entry);
        self.entries.len() - 1 // Return the index of the newly added entry
    }

    pub fn get_entry(&self, index: usize) -> Option<&EntryInfo> {
        self.entries.get(index)
    }
}

pub struct Scanner {
    entry_manager : EntryManager,
    ring : IoUring,
    // sockets: Vec<i32>
}

impl Scanner {
    pub fn new(entries : u32) -> Self {
        let entry_manager = EntryManager {
            entries: Vec::new(),
        };

        let ring = IoUring::new(entries)
        .expect("Error when creating IoUring instance");

        // let mut sockets = Vec::new();
        // for _ in 0..entries {
        //     let sckt = socket(
        //         AddressFamily::Inet,
        //         SockType::Stream,
        //         SockFlag::SOCK_NONBLOCK,
        //         None,
        //     ).expect("TCP socket creation failed");

        //     sockets.push(sckt);
        // }

        // Return the Scanner instance
        Scanner {
            entry_manager,
            ring,
            // sockets
        }
    }
    
    pub fn scan (
        &mut self,
        // mut ring : IoUring,
        ip_start : &str,
        ports : Vec<u16>,
        subnet_len: u8,
        mut chunk_size: usize,
    ) -> Result<(), io::Error> {
    
        let ip_itr : Vec<&str> = ip_start.splitn(4, ".").collect();
        let mut ip_bytes_start : [u8; 4] = [0; 4];
        for i in 0..4 {
            ip_bytes_start[i] = ip_itr[i].parse::<u8>().unwrap();
        }
        
        let ip_range = Ipv4Net::new(
            Ipv4Addr::from(ip_bytes_start), 
            subnet_len
        )
        .expect("Ip range creation failed")
        .hosts()
        .collect::<Vec<Ipv4Addr>>();
    
        // ip_range.reverse();
    
        let chunks = ip_range.chunks(chunk_size);
    
        // TODO: Might change the nesting order here
        for ip_chunk in chunks {
            for port in &ports {
                // The last chunk generally has a shorter length, so we make an exception for it
                if ip_chunk.len() != chunk_size {
                    chunk_size = ip_chunk.len();
                }

                self.connect_batch(ip_chunk, port);
    
                // connect(sckt, addr, &mut ring)
                // .expect(format!("Connection failed to host {}", addr.to_string()).as_str());
            }
        }
    
        // nix::fcntl::fcntl(sckt, nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK)).expect("Failed to set socket to non-blocking");
    
        Ok(())
    }

    fn connect_batch (
        &mut self,
        chunk : &[Ipv4Addr],
        port: &u16,
        // ring : &mut IoUring,
    ) {
        // let mut sckt_idx = 0;

        for ip in chunk{
            let ip_bytes = ip.clone().octets();
            let port_ = port.clone();

            let addr = SockaddrIn::new(
                ip_bytes[0],
                ip_bytes[1],
                ip_bytes[2],
                ip_bytes[3],
                port_,
            );

            // FIXME: Move the code so that the sockets (num of chunk size) are reusable for every chunk
            // let sckt = socket(
            //     AddressFamily::Inet,
            //     SockType::Stream,
            //     SockFlag::SOCK_NONBLOCK,
            //     None,
            // ).expect("TCP socket creation failed");

            // let sckt = *self.sockets.get(sckt_idx).expect("");
            // sckt_idx += 1;
            // println!("Socket: {}, Socket index: {}", sckt, sckt_idx);

            self.connect(&addr)
            .expect(format!("Error while connecting to adress: {}", addr.to_string()).as_str());
        }    

        self.ring.submit_and_wait(chunk.len() * 3)
        .expect("Error submitting to submission queue");

        // let completion = ring.completion().collect::<Vec<io_uring::cqueue::Entry>>();

        // Get CQE results
        for _ in 0..chunk.len() * 3 {
            let cqe: io_uring::cqueue::Entry = self.ring.completion().next().expect("Completion queue is empty");

            // Retrieve the entry index of the completion
            let index = cqe.user_data();

            let entry_info = self.entry_manager.get_entry(index as _)
            .expect("Error when retrieving entry from vector");

            // TODO: Might move this checking section to another function

            // println!("{:?}", chunk);
            // If it is defined as a Connect opcode
            match entry_info.op_type {
                0 => {
                    // Connect opcode completion
                    if cqe.result() >= 0 {
                        println!("Connection established to: {}", entry_info.ip);
                    } else {
                        println!("Connection failed: {} , Error code: {}", entry_info.ip, cqe.result());
                    }
                }
                // Can handle other op types here
                _ => {
                    // println!("Connection to: {} resulted with code: {}, op: {}", entry_info.ip, cqe.result(), entry_info.op_type);
                }
    
            } 
        }
    }

    /*
    * TCP connect to a single port
    */
    fn connect (
        &mut self,
        // sckt : i32,
        addr : &SockaddrIn,
        // ring : &mut IoUring,
        ) -> Result<(), Error> {

        let sckt = socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_NONBLOCK,
            None,
        ).expect("TCP socket creation failed");

        let addr = Rc::new(addr.to_owned());

        let op_connect_index = self.entry_manager.add_entry(
            Rc::new(*addr),
            0,
            sckt.as_raw_fd(),
        );

        let op_timeout_index = self.entry_manager.add_entry(
            Rc::new(*addr),
            1,
            sckt.as_raw_fd()
        );

        let op_close_index = self.entry_manager.add_entry(
            Rc::new(*addr),
            2,
            sckt.as_raw_fd(),
        );


        // Build the Connect opcode to establish connection
        let op_connect: squeue::Entry = opcode::Connect::new(
            Fd(sckt.as_raw_fd()),
            addr.as_ptr(),
            addr.len()
        )
        .build()
        .flags(squeue::Flags::IO_LINK)
        .user_data(op_connect_index as u64);

        // Build the LinkTimeout opcode to add timeout feature
        let timespec = Timespec::new().nsec(50); // TODO: Parameterize
        let op_timeout: squeue::Entry = opcode::LinkTimeout::new(
            &timespec
        )
        .build()
        .flags(squeue::Flags::IO_LINK)
        .user_data(op_timeout_index as u64);

        let op_close = opcode::Close::new(Fd(sckt.as_raw_fd()))
        .build()
        .user_data(op_close_index as u64);

        let ops = [op_connect, op_timeout, op_close];

        unsafe {
            self.ring.submission()
            .push_multiple(&ops)
            .expect("Failed to push operations, submission queue is full");

            // ring.submission()
            // .push(&op_connect)
            // .expect("Failed to push Connect to submission queue, queue is full");
            // ring.submission()
            // .push(&op_timeout)
            // .expect("Failed to push LinkTimeout to submission queue, queue is full");
            // ring.submission()
            // .push(&op_close)
            // .expect("Failed to push Close to submission queue, queue is full");
        }

        Ok(())
    }

    // A function to make the outputs prettier
    fn parse_result (result : &str) {
        todo!()
    }
}

// TODO: 1- Open sockets and store them (OK)
// TODO: 2- In connect_batch, change the for loop to loop over these same sockets for every chunk
fn open_sockets (chunk_size : usize) -> Result<Vec<i32>, Error> {
    let mut sockets: Vec<i32> = vec![];

    for i in 0..chunk_size {
        let sckt = socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_NONBLOCK,
            None,
        ).expect("TCP socket creation failed");

        sockets.push(sckt);
    }

    Ok(sockets)
}


// TODO: Needs ring parameter
fn send_tcp_packet (
    sckt : i32
) -> Result<i32, Error> {

    todo!();
    // const MSG : &str = "GET / HTTP/1.1\r\n Host:localhost";
    const MSG : &str = "Hello, server";

    // let mut tx_buffer : Vec<i32> = vec![0; 128];
    let mut tx_buffer: [u8; 128] = [0; 128];
    tx_buffer[..MSG.len()].copy_from_slice(MSG.as_bytes());

    let send_buffer = iovec {
        iov_base: tx_buffer.as_mut_ptr() as *mut libc::c_void,
        iov_len: tx_buffer.len(),
    };
    // Craft the opcode to send and receive the data to the socket
    let op_send: squeue::Entry = opcode::Write::new(
        Fd(sckt.as_raw_fd()),
        send_buffer.iov_base.cast::<u8>(),
        send_buffer.iov_len as u32,
    ).build();

    // TODO: Temporary, will merge the ring
    let mut ring = IoUring::new(8).expect("Error when creating io_uring");
    
    unsafe {
        ring.submission()
        .push(&op_send)
        .expect("Submission queue is full");
    }

    ring.submit_and_wait(1).expect("io_uring submission error");

    let cqe = ring.completion().next().expect("Completion queue is empty");
    let send_result = cqe.result();
    if send_result < 0 {
        panic!("Error sending message : {}", send_result)
    }
    println!("Bytes sent : {}", send_result);

    Ok(send_result)

}


fn read_response (
    ring: &mut IoUring,
    sckt: i32,
    buffer: &mut [u8],
) {
    todo!();
    // let mut rx_buffer: Vec<u8> = vec![0; 1024];

    // let rcv_buffer = iovec {
    //     iov_base: rx_buffer.as_mut_ptr() as *mut libc::c_void, 
    //     iov_len: rx_buffer.len(),
    // };

    let read_e = opcode::Read::new(
        Fd(sckt.as_raw_fd()),
        buffer.as_mut_ptr(),
        buffer.len() as u32
    ).build();

    unsafe {
        ring.submission().push(&read_e).expect("Submission queue is full");
    };
}


// Input format is "ip/range" like "192.168.1.0/24"
fn parse_subnet (
    subnet_str : &str,
) -> SubnetInfo {
    let mut ip_itr = subnet_str.splitn(2, "/");
    let ip_start = ip_itr.next().unwrap();
    let subnet_len = ip_itr.next().unwrap().parse::<u8>().unwrap();

    // let ports = ports_str.split(",")
    // .map(|s| s.to_string()
    // .parse::<u16>()
    // .expect("Parse error"))
    // .collect();

    // Subnet specifier must be between 0 and 32
    if subnet_len > 32 {
        panic!("Subnet length {} is out of bounds [0,32]", subnet_len);
    }

    let subnet_info: SubnetInfo = SubnetInfo {
        ip_start: ip_start.to_string(),
        subnet_len: subnet_len,
    };

    return subnet_info
}


// Check if the used opcodes are supported for the current kernel
fn check_supported () {
    let mut probe = Probe::new();

    let ring = IoUring::new(8).expect("");
    let _ = ring.submitter().register_probe(&mut probe);

    let connect_supported = probe.is_supported(io_uring::opcode::Connect::CODE);
    let link_timeout_supported = probe.is_supported(io_uring::opcode::LinkTimeout::CODE);

    if !connect_supported {
        panic!("The operation \"Connect\" is not supported by the kernel")
    }
    if !link_timeout_supported {
        panic!("The operation \"LinkTimeour\" is not supported by the kernel")
    }
}


fn parse_ports (filename: &str) -> Result<Vec<u16>, io::Error> {
    let file = File::open(filename)?;
    let reader = io::BufReader::new(file);
    let mut port_list = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let ports: Vec<u16> = line
            .split(',')
            .flat_map(|port_str| {
                if port_str.contains('-') {
                    // Parse range
                    let range: Result<Vec<u16>, _> = port_str
                        .split('-')
                        .map(|num| u16::from_str(num).map_err(|e| e.to_string()))
                        .collect();

                    match range {
                        Ok(range) => {
                            let start = range.get(0).unwrap();
                            let end = range.get(1).unwrap();
                            (*start..=*end).collect()
                        },
                        Err(err) => {
                            eprintln!("Error parsing range: {}", err);
                            Vec::new()
                        }
                    }
                } else {
                    // Parse single port
                    match u16::from_str(port_str) {
                        Ok(port) => vec![port],
                        Err(err) => {
                            eprintln!("Error parsing port: {}", err);
                            Vec::new()
                        }
                    }
                }
            })
            .collect();

        port_list.extend(ports);
    }

    Ok(port_list)
}


fn main() {
    // TODO: Remove
    // use sudo;

    let filename = "src/data/top1000TCP.txt";
    

    // println!("ports: {:?}", ports);
    // println!("ports len: {}", ports.len());

    // Take arguments
    let args : Vec<String> = env::args().collect();

    check_supported(); // Check if the kernel supports io_uring operations used in the program

    // let running_as = sudo::check();
    // println!("{:?}", running_as);
    // sudo::escalate_if_needed().expect("Error when escalating privileges");

    // Get parameters
    let subnet_str = args.get(1).expect("Argument error: Wrong ip range format");
    // let ports_str = args.get(2).expect("Argument error: Wrong ports format");
    
    let subnet_info = parse_subnet(subnet_str);
    
    
    let ip_start = subnet_info.ip_start.clone();
    // let ports = subnet_info.ports.clone();
    let ports = parse_ports(filename)
    .expect("Error when parsing ports");
    let subnet_len = subnet_info.subnet_len.clone();

    let size = 128; // TODO: Take from args

    let mut scanner = Scanner::new(size * 3);

    scanner.scan(
        &ip_start,
        ports,
        subnet_len,
        size as usize,
    ).expect("Error scanning");


    // TODO: For testing
    // for i in 0..scanner.entry_manager.entries.len() {
    //     let entry = &scanner.entry_manager.entries[i];
    //     println!("{}: {}, {}, {}", i, entry.ip, entry.fd, entry.op_type);

    //     if i > 100 {break;}
    // }

    // send_tcp_packet(ip_port);
    
}


// TODO: Add a full port scan 0-65535