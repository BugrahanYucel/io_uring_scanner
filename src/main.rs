use io_uring::Submitter;
use io_uring::types::Timespec;
use io_uring::{cqueue, squeue, opcode, types::Fd, IoUring, Probe};
use nix::Error;

use std::io::{self, Write};
use std::os::fd::{RawFd, AsRawFd};
use std::str::{from_utf8, FromStr};
use std::{env, array};

use nix::sys::socket::{socket, AddressFamily, SockFlag, SockType, SockaddrLike, SockaddrIn, SockProtocol};
use libc::{self, iovec};

use std::net::{Ipv4Addr};

use ipnet::{IpSubnets, Ipv4Subnets, IpAddrRange, Ipv4AddrRange, Ipv4Net};

// TODO: Parse arguments with flags using clap, looks good
// use clap::{Arg, App};

#[derive(Clone)]
struct SubnetInfo {
    ip_start : String,
    ports : Vec<u16>,
    subnet_len : u8,
} 

fn scan (
    mut ring : IoUring,
    ip_start : &str,
    ports : Vec<u16>,
    subnet_len: u8,
    mut chunk_size: usize,
) -> Result<(), io::Error> {

    // Parse the ip and port
    // let mut ip_port_itr = ip_port.splitn(2, ":");
    // let ip = std::net::IpAddr::from_str(ip_port_itr.next().expect("")).expect("");
    // let port = ip_port_itr.next().expect("").parse::<u16>().unwrap();
    
    let ip_itr : Vec<&str> = ip_start.splitn(4, ".").collect();
    let mut ip_bytes_start : [u8; 4] = [0; 4];
    for i in 0..4 {
        ip_bytes_start[i] = ip_itr[i].parse::<u8>().unwrap();
    }
    
    let ip_range = Ipv4Net::new(
        Ipv4Addr::from(ip_bytes_start), subnet_len
    )
    .expect("Ip range creation failed")
    .hosts()
    .collect::<Vec<Ipv4Addr>>();

    // let mut chunk_size = 16;
    let chunks = ip_range.chunks(chunk_size);

    // TODO: Might change the nesting order here
    for ip_chunk in chunks {
        for port in &ports {
            // The last chunk generally has a shorter length, so we make an exception for it
            if ip_chunk.len() != chunk_size {
                chunk_size = ip_chunk.len();
            }

            connect_batch(ip_chunk, port, &mut ring);



            // connect(sckt, addr, &mut ring)
            // .expect(format!("Connection failed to host {}", addr.to_string()).as_str());
        }
    }

    // nix::fcntl::fcntl(sckt, nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK)).expect("Failed to set socket to non-blocking");

    Ok(())
}

// TODO: Check if it enters every parameter correctly
fn connect_batch (
    chunk : &[Ipv4Addr],
    port: &u16,
    ring : &mut IoUring,
) {
    for ip in chunk {
        let ip_bytes = ip.clone().octets();
        let port_ = port.clone();

        let addr = SockaddrIn::new(
            ip_bytes[0],
            ip_bytes[1],
            ip_bytes[2],
            ip_bytes[3],
            port_,
        );

        // println!("addr in connect_batch: {:?}", addr);
        // println!("addr: {:?}", addr);

        // TODO: Move the code so that the sockets (num of chunk size) are reusable for every chunk
        let sckt = socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_NONBLOCK,
            None,
        ).expect("TCP socket creation failed");

        connect(sckt, addr, ring)
        .expect(format!("Error while connecting to adress: {}", addr.to_string()).as_str());
    }    
    
    println!("{}", chunk.len());
    ring.submit_and_wait(chunk.len())
    .expect("Error submitting to submission queue");

    for i in 0..chunk.len() {
        let addr = chunk[i];
        let cqe = ring.completion().next().expect("Completion queue is empty");
        
        if cqe.result() >= 0 {
            println!("Connection established to: {} on port {}", addr.to_string(), port);
        } else {
            println!("Connection failed: {}:{} , Error code: {}", addr.to_string(), port, cqe.result());
        }
    }

}

/*
* TCP connect to a single port
*/
fn connect (
    sckt : i32,
    addr : SockaddrIn,
    ring : &mut IoUring
    ) -> Result<(), Error> {

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

    unsafe {
        ring.submission()
        .push(&op_connect)
        .expect("Failed to push connect to submission queue");
    }

    // TODO: move the result checking here and test the results

    Ok(())
}


fn open_sockets () -> Result<Vec<SockaddrIn>, Error> {
    return todo!()
}


// TODO: Needs ring parameter
fn send_tcp_packet (
    sckt : i32
) -> Result<i32, Error> {
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
    ports_str : String
) -> SubnetInfo {
    let mut ip_itr = subnet_str.splitn(2, "/");
    let ip_start = ip_itr.next().unwrap();
    let subnet_len = ip_itr.next().unwrap().parse::<u8>().unwrap();
    let ports = ports_str.split(",")
    .map(|s| s.to_string()
    .parse::<u16>()
    .expect("Parse error"))
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

fn test (ring :&mut IoUring) {
    let ip_range = vec![
        Ipv4Addr::new(127, 0, 0, 1),
        Ipv4Addr::new(127, 0, 0, 2),
        Ipv4Addr::new(127, 0, 0, 3),
        Ipv4Addr::new(127, 0, 0, 4),
        Ipv4Addr::new(127, 0, 0, 5),
        Ipv4Addr::new(127, 0, 0, 6),
        Ipv4Addr::new(127, 0, 0, 7),
        Ipv4Addr::new(127, 0, 0, 8),
    ];

    let port = 9001;
    let addr = SockaddrIn::new(
        127,
        0,
        0,
        1,
        port.clone(),
    );

    let sckt = socket(
        AddressFamily::Inet,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK,
        None,
    ).expect("TCP socket creation failed");


    // let _ = connect_batch(ip_range.as_slice(), &9001, ring);
    let _ = connect(sckt, addr, ring);

    let chunk_size = 1;
    let _ = ring.submit_and_wait(chunk_size)
    .expect("Error submitting to submission queue");
    
    for i in 0..chunk_size {
        let cqe = ring.completion().next().expect("Completion queue is empty");

        if cqe.result() >= 0 {
            println!("Connection established");
        } else {
            println!("Connection failed to: , Error code: {}", cqe.result());
    }

}
    
}

fn main() {
    // Take arguments
    let args : Vec<String> = env::args().collect();

    
    let subnet_str = args.get(1).expect("Argument error: Wrong ip range format");
    let ports_str = args.get(2).expect("Argument error: Wrong ports format");
    
    let subnet_info = parse_subnet(subnet_str, ports_str.to_string());
    
    // println!("ip_start: {}\nports: {:?}\nsubnet_len: {}", 
    // subnet_info.ip_start,
    // subnet_info.ports,
    // subnet_info.subnet_len
    // );

    let size = 16; // TODO: Take from args
    let ring = IoUring::new(size).expect("Ring creation failed");

    // test(&mut ring);

    let ip_start = subnet_info.ip_start.clone();
    let ports = subnet_info.ports.clone();
    let subnet_len = subnet_info.subnet_len.clone();

    scan(
        ring,
        &ip_start,
        ports,
        subnet_len,
        size as usize
    ).expect("Error when creating TCP socket");

    // send_tcp_packet(ip_port);
    
}
