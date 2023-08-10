use std::io::{self, Error};
use std::os::fd::{AsRawFd, RawFd};
use std::rc::Rc;

use nix::sys::socket::{socket, AddressFamily, SockFlag, SockType, SockaddrLike, SockaddrIn};
use libc::{self, iovec};

use io_uring::{squeue, opcode, types::Fd, IoUring, types::Timespec};
use io_uring::Probe;
use nix::unistd;

use std::net::Ipv4Addr;
use ipnet::Ipv4Net;

use std::fs::File;
use std::io::BufRead;
use std::str::FromStr;
use std::time::Instant;

use clap::{ArgAction, Command, Arg};

#[derive(Clone)]
struct SubnetInfo {
    ip_start : String,
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
    entries : Vec<Option<EntryInfo>>,
}

impl EntryManager {
    pub fn new() -> Self {
        EntryManager { entries: Vec::new() }
    }

    pub fn add_entry(&mut self, ip: Rc<SockaddrIn>, op_type: u8, fd: RawFd) -> usize {
        let entry = EntryInfo { ip, op_type, fd };
        self.entries.push(Some(entry));
        self.entries.len() - 1 // Return the index of the newly added entry
    }

    pub fn get_entry(&self, index: usize) -> Option<&EntryInfo> {
        self.entries.get(index).expect("The entry at index does not exist").as_ref()
    }

    pub fn free_entry (&mut self, index : u64) {
        self.entries[index as usize] = None;
    }
}

pub struct Scanner {
    entry_manager : EntryManager,
    ring : IoUring,
    num_step : u8,
}

impl Scanner {
    pub fn new(entries : u32, num_step : u8) -> Self {
        let entry_manager = EntryManager {
            entries: Vec::new(),
        };

        let ring = IoUring::new(entries * num_step as u32)
        .expect("Error when creating IoUring instance");

        // Return the Scanner instance
        Scanner {
            entry_manager : entry_manager,
            ring : ring,
            num_step : num_step as _,
        }
    }
    
    pub fn scan (
        &mut self,
        ip_range : Vec<Ipv4Addr>,
        ports : Vec<u16>,
    ) -> Result<(), io::Error> {

        let mut curr_ip_idx : usize = 0;
        let mut curr_port_idx : usize = 0;

        // TODO: Arrange those redundant checks
        while curr_ip_idx < ip_range.len() && curr_port_idx < ports.len() {
            
            let mut pushed = 0;
            
            // Push entries
            let capacity = self.ring.submission().capacity();

            while capacity - self.num_step as usize >= self.ring.submission().len() 
            && curr_ip_idx < ip_range.len()
            {

                let curr_ip = ip_range.get(curr_ip_idx).unwrap();
                let curr_port = ports.get(curr_port_idx).unwrap();

                let sckt = self.create_socket().as_raw_fd();

                let ip_bytes = curr_ip.clone().octets();
                let port = curr_port.clone();

                let addr = SockaddrIn::new(
                    ip_bytes[0],
                    ip_bytes[1],
                    ip_bytes[2],
                    ip_bytes[3],
                    port,
                );

                let _ = self.push_connect(&addr, sckt);
                
                pushed += 1;

                curr_port_idx += 1;
                if curr_port_idx >= ports.len() {
                    curr_ip_idx += 1;
                    curr_port_idx = 0;
                }
            }

            // submit_and wait() is the best approach
            let _ = self.ring.submit_and_wait(pushed * self.num_step as usize)?;
            // let _ = self.ring.submit();

            // Consume results
            while !self.ring.completion().is_empty() {
                let cqe: io_uring::cqueue::Entry = self.ring.completion().next().expect("Completion queue is empty");

                // Retrieve the entry index of the completion
                let index = cqe.user_data();

                let entry_info = self.entry_manager.get_entry(index as _)
                .expect("Error when retrieving entry from vector");

                match entry_info.op_type {
                    0 => {
                        // Connect opcode completion
                        if cqe.result() >= 0 {
                            println!("Connection established to: {}", entry_info.ip);
                        } else {
                            // println!("Connection failed: {} , Error code: {}", entry_info.ip, cqe.result());
                            // if cqe.result() == -125 {
                            //     println!("Connection canceled");
                            // }
                        }
                    }
                    // Can handle other op types here
                    _ => {
                        // println!("Connection to: {} resulted with code: {}, op: {}", entry_info.ip, cqe.result(), entry_info.op_type);
                    }
        
                }

                // Close dangling sockets in case of operation cancel
                if cqe.result() == -libc::ECANCELED {
                    // println!("Freeing : {} for {}", entry_info.fd, entry_info.ip);
                    let _ = unistd::close(entry_info.fd);
                }

                self.entry_manager.free_entry(index);
            }
        }
        
        Ok(())
    }


    /*
    * TCP connect to a single port
    */
    fn push_connect (
        &mut self,
        addr : &SockaddrIn,
        sckt : i32,
        ) -> Result<(), Error> {

        let addr = Rc::new(addr.to_owned());

        let op_connect_index = self.entry_manager.add_entry(
            Rc::clone(&addr),
            0,
            sckt,
        );

        let op_timeout_index = self.entry_manager.add_entry(
            Rc::clone(&addr),
            1,
            sckt,
        );

        let op_close_index = self.entry_manager.add_entry(
            Rc::clone(&addr),
            2,
            sckt,
        );


        // Build the Connect opcode to establish connection
        let op_connect: squeue::Entry = opcode::Connect::new(
            Fd(sckt),
            addr.as_ptr(),
            addr.len()
        )
        .build()
        .flags(squeue::Flags::IO_LINK)
        .user_data(op_connect_index as u64);

        // TODO: Parameterize timeout duration
        // let timespec = Timespec::new().nsec(1000000 * 500); // 0.001 seconds * X
        let timespec = Timespec::new().sec(1);

        let op_timeout: squeue::Entry = opcode::LinkTimeout::new(
            &timespec
        )
        .build()
        .flags(squeue::Flags::IO_LINK)
        .user_data(op_timeout_index as u64);

        let op_close = opcode::Close::new(
            Fd(sckt),
        )
        .build()
        .user_data(op_close_index as u64);

        let ops = [
            op_connect,
            op_timeout,
            op_close
            ];

        unsafe {
            self.ring.submission()
            .push_multiple(&ops)
            .expect("Failed to push operations, submission queue is full");
        }

        Ok(())
    }

    // A function to make the outputs prettier
    fn parse_result (result : &str) {
        todo!()
    }

    fn create_socket (&self) -> RawFd {
        socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_NONBLOCK,
            None,
        ).expect("TCP socket creation failed")
    }

    fn read_response (
        &mut self,
        addr : &SockaddrIn,
        buffer: &mut [u8],
    ) {
        const BUFFER_SIZE: usize = 1024;
        let mut buffer: [u8; BUFFER_SIZE] = [0; BUFFER_SIZE];
    
        // let mut rx_buffer: Vec<u8> = vec![0; 1024];
    
        // let rcv_buffer = iovec {
        //     iov_base: rx_buffer.as_mut_ptr() as *mut libc::c_void, 
        //     iov_len: rx_buffer.len(),
        // };

        let sckt = self.create_socket();
    
        let op_read_index = self.entry_manager.add_entry(
            Rc::new(*addr),
            2,
            sckt.as_raw_fd(),
        );
        let read_e = opcode::Read::new(
            Fd(sckt.as_raw_fd()),
            buffer.as_mut_ptr(),
            buffer.len() as u32
        ).build()
        .user_data(0);
    
        unsafe {
            self.ring.submission().push(&read_e).expect("Submission queue is full");
        };
    }
}


// Input format is "ip/range" like "192.168.1.0/24"
fn parse_subnet (
    subnet_str : &str,
) -> SubnetInfo {
    let mut ip_itr = subnet_str.splitn(2, "/");
    let ip_start = ip_itr.next().unwrap();

    // ip_itr.next().unwrap().parse::<u8>().unwrap();
    // println!("a: {}", a);
    let subnet_len = match ip_itr.next() {
        Some(v) => v.parse::<u8>().unwrap(),
        None => 32,
    };
    // println!("subnet_len: {}", a);

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


fn get_ip_range (
        ip_start : &str,
        subnet_len: u8,
) -> Vec<Ipv4Addr> {
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

    return ip_range;
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
        panic!("The operation \"LinkTimeout\" is not supported by the kernel")
    }
}


fn parse_ports (line : String
) -> Result<Vec<u16>, io::Error> {
    let mut port_list = Vec::new();

    let line = line;
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

    Ok(port_list)


}


fn parse_ports_from_file (filename: &str) -> Result<Vec<u16>, io::Error> 
{
    let file = File::open(filename)?;
    let reader = io::BufReader::new(file);
    let mut port_list = Vec::new();


    for line in reader.lines() {
        let a = parse_ports(line.unwrap()).unwrap();

        port_list.extend(a);
    }

    Ok(port_list)
}


fn main() {
    let tcp_top_1000 = "src/data/top1000TCP.txt";
    
    check_supported(); // Check if the kernel supports io_uring operations used in the program

    let matches = Command::new("App")
    .version("0.16")
    .author("Bugrahan")
    .arg(
        Arg::new("ip")
        .index(1)
        .required(true)
        .value_name("ip_range")
        .help("usage: io_uring_scanner IP[/SUBNET] [OPTIONS]")
    )
        .arg(Arg::new("port")
            .short('p')
            .long("port")
            .action(ArgAction::Append)
    )
        .arg(Arg::new("file_name")
            .short('f')
            .long("file")
            .action(ArgAction::Append)
    )
    .get_matches();


    let ports : Vec<u16> = match matches.get_one::<String>("port") {
        None => {
            parse_ports_from_file(tcp_top_1000)
            .expect("Error when parsing ports from file")
        },
        Some(_) => {
            parse_ports(matches.get_one::<String>("port")
            .expect("No ports are supplied")
            .to_string())
            .expect("Error when parsing ports from arguments")
        }
    };

    let subnet_str = matches.get_one::<String>("ip")
    .expect("Argument error: Wrong ip range format");
    
    let subnet_info = parse_subnet(subnet_str);
    
    let ip_start = subnet_info.ip_start.clone();
    let subnet_len = subnet_info.subnet_len.clone();

    println!("subnet_len: {}", subnet_info.subnet_len);

    let ip_range = match subnet_info.subnet_len {
        32 => vec![Ipv4Addr::from_str(ip_start.as_str()).expect("Wrong IP format")],
        _ => get_ip_range(&ip_start, subnet_len)
    };
    
    let chunk_size: u32 = 2048; // TODO : Take from args

    let mut scanner = Scanner::new(chunk_size / 3, 3);

    let start = Instant::now();
    scanner.scan(
        ip_range,
        ports,
        // chunk_size as usize,
    ).expect("Error scanning");
    let duration = Instant::now() - start;

    println!("TCP connect scan finished in {:5} seconds.", duration.as_millis() as f64 / 1000.0);


}

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
