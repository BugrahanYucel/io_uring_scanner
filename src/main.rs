use io_uring::Submitter;
use io_uring::{cqueue, squeue, opcode, types::Fd, IoUring, Probe};
use nix::Error;

use std::io::{self, Write};
use std::os::fd::{RawFd, AsRawFd};
use std::str::{from_utf8, FromStr};
use std::{env, array};

use nix::sys::socket::{socket, AddressFamily, SockFlag, SockType, SockaddrLike, SockaddrIn, SockProtocol};
use libc::{self, iovec};

use std::net::{Ipv4Addr};

use ipnet::{IpSubnets, Ipv4Subnets};

// TODO: Is using static lifetime OK here?
#[derive(Clone)]
struct SubnetInfo {
    ip_start : String,
    ports : Vec<u16>,
    subnet_len : u8,
}

struct Ring {
    entries : u32,
    rx_size : u32,
}

impl Ring {
    // pub fn new (ring : IoUring) -> Result<Self, Error> {
        // let mut ring = IoUring::new(8).expect("Error when creating io_uring");
        
        // let sq: io_uring::SubmissionQueue = ring.submission();
        // let cq: io_uring::CompletionQueue = ring.completion();

        // Ok(Self {})
    // }


    // pub fn submit_sq (&mut self, entry : squeue::Entry) -> Result<(), Error> {
    //     unsafe {
    //         self.sq.push(&entry).expect("Submission queue is full");
    //     }
    //     Ok(())
    // }

    // pub fn submit_and_wait(&self, num : i32) {
    //     // ring.submit_and_wait(1).expect("Completion queue is empty");
        
    // }

    // pub fn return_cqe (&mut self, entry : squeue::Entry) -> Result<(), Error>{

    //     let cqe = self.cq.next().expect("Completion queue is empty");
    //     let connect_result = cqe.result();
    //     if connect_result < 0 {
    //         panic!("Completion queue failed to retrieve cqe")
    //     }

    //     let result = cqe.result();

    //     Ok(())
    // }

} 

fn scan (
    ring : IoUring,
    ip_start : &str,
    ports : Vec<u16>,
    subnet_len: u8
) -> Result<i32, io::Error> {

    let max_ip = "255.255.255.255";

    // TODO: Loop here
    let subnets: IpSubnets = IpSubnets::from(Ipv4Subnets::new(
        ip_start.parse().unwrap(),
        max_ip.parse().unwrap(),
        subnet_len
    ));

    for ip in subnets {
        println!("{}", ip.to_string());
    }

    // println!("Trying to connect to: {}", ip_port);

    // TODO: Uncomment
    // Parse the ip and port
    // let mut ip_port_itr = ip_port.splitn(2, ":");
    // let ip = std::net::IpAddr::from_str(ip_port_itr.next().expect("")).expect("");
    // let port = ip_port_itr.next().expect("").parse::<u16>().unwrap();
    
    // let ip_str = ip.to_string();
    // let ip_itr : Vec<&str> = ip_str.splitn(4, ".").collect();

    // let mut ip_bytes : [u8; 4] = [0; 4];
    // for i in 0..4 {
    //     ip_bytes[i] = ip_itr[i].parse::<u8>().unwrap();
    // }

    // TODO: Uncomment
    // println!("Trying: {:?}", ip_bytes);

    // let addr = SockaddrIn::new(
    //     ip_bytes[0],
    //     ip_bytes[1],
    //     ip_bytes[2],
    //     ip_bytes[3],
    //     port
    // );

    // Create a socket
    let sckt = socket(
        AddressFamily::Inet,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK, 
        None,
    ).expect("TCP socket creation failed");

    // Set the socket to non-blocking mode
    // TODO: Chek if as_raw_fd() method is needed or not
    // nix::fcntl::fcntl(sckt, nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK)).expect("Failed to set socket to non-blocking");

    // TODO: Do this with a range of ips and ports
    // connect(sckt, addr, ring).expect("Error when trying to connect");

    Ok(sckt)

}

/*
* TCP connect to a single port
*/
fn connect (
    sckt : i32, 
    addr : SockaddrIn,
    mut ring : IoUring
    ) -> Result<i32, Error> {

    // Register opcode to establish connection
    let op_connect: squeue::Entry = opcode::Connect::new(
        Fd(sckt.as_raw_fd()),
        addr.as_ptr(),
        addr.len()
    )
    .build();
    // .flags(squeue::Flags::IO_LINK);

    // TODO: include timeout
    // let connect_timeout = Timespec::new().sec(5);
    // let op_connect_timeout = opcode::LinkTimeout::new(&connect_timeout).build();
    // let ops = [op_connect, op_connect_timeout];
    
    unsafe {
        ring.submission()
        .push(&op_connect)
        .expect("Failed to push connect to submission queue");
    }
    let _ = ring.submit();

    let cqe = ring.completion().next().expect("Completion queue is empty");

    if cqe.result() >= 0 {
        println!("Connection established!");
    } else {
        println!("Connection failed. Error code: {}", cqe.result());
    }

    Ok(cqe.result())

}

fn send_tcp_packet (
    sckt : i32
) -> Result<i32, Error> {
    const MSG : &str = "GET / HTTP/1.1\r\n Host:localhost";
    // const MSG : &str = "GET /.bashrc HTTP/1.1";

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
fn parse_subnet (subnet_str : &str, ports_str : String) -> SubnetInfo
{
    let mut ip_itr = subnet_str.splitn(2, "/");
    let ip_start = ip_itr.next().unwrap();
    let subnet_len = ip_itr.next().unwrap().parse::<u8>().unwrap();
    let ports = ports_str.split(",")
    .map(|s| s.to_string()
    .parse::<u16>().expect("Parse error"))
    .collect();

    // Subnet specifier must be between 0 and 32
    if subnet_len > 32 {
        panic!("Subnet length {} is out of bounds [0,32]", subnet_len);
    }

    let subnet_info: SubnetInfo = SubnetInfo {
        ip_start: ip_start.to_string(),
        ports: ports,
        subnet_len: subnet_len,
    };

    return subnet_info
}

fn main() {
    // Take arguments
    let args : Vec<String> = env::args().collect();

    // let ring = Ring::new(io_uring).expect("Ring struct creation failed");
    
    let subnet_str = args.get(1).expect("Argument error: Wrong ip range format");
    let ports_str = args.get(2).expect("Argument error: Wrong ports format");
    
    let subnet_info = parse_subnet(subnet_str, ports_str.to_string());
    
    println!("ip_start: {}\nports: {:?}\nsubnet_len: {}", 
    subnet_info.ip_start,
    subnet_info.ports,
    subnet_info.subnet_len
    );

    let ring = IoUring::new(8).expect("Ring creation failed");
    scan(
        ring,
        &subnet_info.ip_start,
        subnet_info.ports,
        subnet_info.subnet_len,
    ).expect("Error when creating TCP socket");

    // send_tcp_packet(ip_port);
    
}

// TODO: Move ip parsing operations to another function
// TODO: Store the ring anywhere else (Mandatory)
// TODO: Move opcode creations into their own expressions
