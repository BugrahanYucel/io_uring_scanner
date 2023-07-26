// use io_uring::Submitter;
use io_uring::types::Timespec;
use io_uring::{cqueue, squeue, opcode, types::Fd, IoUring, Probe};
use std::io::{self, Write};
// use std::net::{TcpListener, TcpStream, SocketAddr, Ipv4Addr, IpAddr};
use std::os::fd::{RawFd, AsRawFd};
use std::str::{from_utf8, FromStr};

// use std::rc::Rc;
use nix::sys::socket::{socket, AddressFamily, SockFlag, SockType, SockaddrLike, SockaddrIn};
use libc::{self, iovec};

use std::{env, array};

// const BUF_SIZE: usize = 4096;

fn tcp_connect (ip_port : &str) -> Result<i32, io::Error>
{
    println!("Trying to connect to: {}", ip_port);

    // Split the ip_port
    let mut ip_port_itr = ip_port.splitn(2, ":");
    let ip = std::net::IpAddr::from_str(ip_port_itr.next().expect("")).expect("");
    let port = ip_port_itr.next().expect("").parse::<u16>().unwrap();
    
    let ip_str = ip.to_string();
    let ip_itr : Vec<&str> = ip_str.splitn(4, ".").collect();

    let mut ip_bytes : [u8; 4] = [0; 4];
    for i in 0..4 {
        ip_bytes[i] = ip_itr[i].parse::<u8>().unwrap();
    }

    println!("{:?}", ip_bytes);


    // TODO: This might be optimized using opcode::Socket maybe? https://docs.rs/io-uring/latest/io_uring/opcode/struct.Socket.html
    // let socket = match TcpStream::connect(ip_port) {
    //     Ok(socket) => socket,
    //     Err(err) => {
    //         println!("Error connecting to {} : {}", ip, err);
    //         return;
    //     }
    // };

    // ip.to_string().splitn(4, ".");

    // Create a socket
    let sckt = socket(
        AddressFamily::Inet,
        SockType::Stream,
        SockFlag::empty(), 
        None,
    ).expect("TCP socket creation failed");

    let addr = SockaddrIn::new(
        ip_bytes[0],
        ip_bytes[1],
        ip_bytes[2],
        ip_bytes[3],
        port
    );

    let op_connect = opcode::Connect::new(
        Fd(sckt.as_raw_fd()),
        addr.as_ptr(),
        addr.len()
    )
    .build()
    .flags(squeue::Flags::IO_LINK);

    // let connect_timeout = Timespec::new().sec(5);
    // let op_connect_timeout = opcode::LinkTimeout::new(&connect_timeout).build();

    // let ops = [op_connect, op_connect_timeout];
    
    let mut rx_buffer: Vec<u8> = vec![0; 128];
    let fixed_buffer = iovec {
        iov_base: rx_buffer.as_mut_ptr() as *mut libc::c_void, 
        iov_len: rx_buffer.len(),
    };

    let op_recv = opcode::ReadFixed::new(
        Fd(sckt.as_raw_fd()),
        fixed_buffer.iov_base.cast::<u8>(),
        fixed_buffer.iov_len as u32,
        0,
    ).build();

    let mut ring = IoUring::new(8).expect("Error when creating io_uring");



    // unsafe {
    //     ring.submitter()
    //     .register_buffers(&[fixed_buffer])
    //     .expect("Submission queue is full");
    // }

    // Establish a connection
    // Submit the send operation to SQ
    unsafe {
        ring.submission()
        .push(&op_connect)
        .expect("Submission queue is full");
    }

    ring.submit_and_wait(1).expect("io_uring submission error");

    let cqe = ring.completion().next().expect("Completion queue is empty");
    let connect_result = cqe.result();
    if connect_result < 0 {
        panic!("Error connecting to {} : {}", addr, connect_result)
    }
    println!("Connection result for {} : {}", addr, connect_result);

    // Receive the response
    // Submit the ReadFixed operation into SQ
    unsafe {
        ring.submission()
        .push(&op_recv)
        .expect("Submission queue is full");
    }
    ring.submit_and_wait(1).expect("Completion queue is empty");
    let read_result = cqe.result();
    if read_result < 0 {
        panic!("Error receiving response from {} : {}", addr, read_result);
    }

    // println!("Response result for {} : {}", addr, &fixed_buffer.iov_base.read());

    Ok(sckt)

}

fn send_tcp_packet (sckt : i32) 
{
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

}

fn read_response () {
    let mut rx_buffer: Vec<u8> = vec![0; 128 * 128];
    let rcv_buffer = iovec {
        iov_base: rx_buffer.as_mut_ptr() as *mut libc::c_void, 
        iov_len: rx_buffer.len(),
    };


    // Receive the response for the request
    // TODO: Craft the opcode
    // unsafe {
    //     let op_read: squeue::Entry = opcode::Read::new(
    //         Fd(sckt.as_raw_fd()),
    //         rcv_buffer.iov_base.cast::<u8>(),
    //         rcv_buffer.iov_len as u32,
    //     ).build();
     
    //     ring.submission()
    //     .push(&op_read)
    //     .expect("Submission queue is full");
    // }
    // ring.submit_and_wait(1).expect("io_uring submission error");
    
    // let cqe = ring.completion().next().expect("Completion queue is empty");
    // let read_result = cqe.result();
    // if read_result < 0 {
    //     panic!("Error reading message : {}", read_result)
    // }
    // println!("Bytes read : {}", read_result);
}

fn main() {
    // Take arguments
    let args : Vec<String> = env::args().collect();

    if args.len() > 2 {
        println!("Please enter a valid address");
        return;
    }

    // let ip_port = "127.0.0.1:8080";
    let ip_port = args.get(1).expect("Argument error");
    
    let sckt = tcp_connect(ip_port).expect("Error when creating TCP socket");
    // let data = ""; // TODO: Parameterize the data
    send_tcp_packet(sckt);
}

// TODO: Move ip parsing operations to another function
// TODO: Store the ring anywhere else (Critical)
// TODO: Move opcode creations into their own expressions

// TODO: Can I send crafted TCP segments?

// chompie
