use io_uring::Submitter;
use io_uring::{cqueue, squeue, opcode, types::Fd, IoUring, Probe};
use nix::Error;

use std::io::{self, Write};
use std::os::fd::{RawFd, AsRawFd};
use std::str::{from_utf8, FromStr};
use std::{env, array};

use nix::sys::socket::{socket, AddressFamily, SockFlag, SockType, SockaddrLike, SockaddrIn, SockProtocol};
use libc::{self, iovec};

use pnet::packet::{Packet, icmp};

struct Ring<'a> {
    sq: &'a io_uring::SubmissionQueue<'a>,
    cq: &'a io_uring::CompletionQueue<'a>,
}

impl<'a> Ring<'a> {
    pub fn new (entries: u32) -> Result<Self, Error> {
        let mut ring = IoUring::new(entries).expect("Error when creating io_uring");
        let sq = ring.submission();
        let cq = ring.completion();

        Ok(Self {sq : &sq, cq: &cq})
        
    }

//     pub fn submission (&self) -> Option<&io_uring::SubmissionQueue<'a>> {
//     }

//     pub fn completion (&self) -> Option<&io_uring::CompletionQueue<'a>> {
//     }

    pub fn submit_sq (&mut self, entry : squeue::Entry) -> Result<(), Error> {
        unsafe {
            self.sq.push(&entry).expect("Submission queue is full");
        }
        Ok(())
    }

    pub fn submit_and_wait(&self, num : i32) {
        // ring.submit_and_wait(1).expect("Completion queue is empty");
        
    }

    pub fn return_cqe (&mut self, entry : squeue::Entry) -> Result<(), Error>{

        let cqe = self.cq.next().expect("Completion queue is empty");
        let connect_result = cqe.result();
        if connect_result < 0 {
            panic!("Completion queue failed to retrieve cqe")
        }

        let result = cqe.result();

        Ok(())
    }

    // TODO: Do I need other methods for buffered I/O operations, 
    // or can I implement them on top of these?
    // I DON'T!

} 

fn tcp_connect (ring : Ring, ip_port : &str) -> Result<i32, io::Error>
{
    println!("Trying to connect to: {}", ip_port);

    // Parse the ip and port
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

    let addr = SockaddrIn::new(
        ip_bytes[0],
        ip_bytes[1],
        ip_bytes[2],
        ip_bytes[3],
        port
    );

    // Create a socket
    let sckt = socket(
        AddressFamily::Inet,
        SockType::Stream,
        SockFlag::empty(), 
        None,
    ).expect("TCP socket creation failed");

    // Set the socket to non-blocking mode
    // TODO: Chek if as_raw_fd() mmethod is needed or not
    nix::fcntl::fcntl(sckt, nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK)).expect("Failed to set socket to non-blocking");
    

    /*
     * Connection
     */

    // Register opcode to establish connection
    let op_connect: squeue::Entry = opcode::Connect::new(
        Fd(sckt.as_raw_fd()),
        addr.as_ptr(),
        addr.len()
    )
    .build();
    // .flags(squeue::Flags::IO_LINK);

    // let connect_timeout = Timespec::new().sec(5);
    // let op_connect_timeout = opcode::LinkTimeout::new(&connect_timeout).build();
    // let ops = [op_connect, op_connect_timeout];
    
    let mut tx_buffer: Vec<u8> = vec![0; 128];
    let fixed_buffer = iovec {
        iov_base: tx_buffer.as_mut_ptr() as *mut libc::c_void, 
        iov_len: tx_buffer.len(),
    };

    let op_recv = opcode::ReadFixed::new(
        Fd(sckt.as_raw_fd()),
        fixed_buffer.iov_base.cast::<u8>(),
        fixed_buffer.iov_len as u32,
        0,
    ).build();

    Ok(sckt)

}

// TODO: Parameterize data
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
) -> Result<u32, Error> {
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
    }

    Ok(())
}


/*
fn craft_icmp (buf : &mut [u8], sequence_number : u16, identifier : u16) -> MutableEchoRequestPacket<>
{
    let mut echo_packet = MutableEchoRequestPacket::new(buf).unwrap();
    echo_packet.set_sequence_number(sequence_number);
    echo_packet.set_identifier(identifier);
    echo_packet.set_icmp_type(IcmpTypes::EchoRequest);
    echo_packet.set_icmp_code(IcmpCode::new(12));

    let echo_checksum = checksum(&IcmpPacket::new(echo_packet.packet()).unwrap());
    echo_packet.set_checksum(echo_checksum);

    println!("echo_packet: {:?}", echo_packet);

    return echo_packet;
    
    // TODO: Send the packet normally, then test it with io_uring
}
*/

/*
fn send_icmp () {

    let packet_size = EchoRequestPacket::minimum_packet_size();
    let mut buf = vec![0; packet_size];

    let icmp_packet = craft_icmp(&mut buf[..], 0, 1);
    let test : &mut str = icmp_packet;

    println!("payload: {:?}", test);
}
*/

fn main() {
    // Take arguments
    let args : Vec<String> = env::args().collect();

    let ring = Ring::new(8);

    // if args.len() > 2 {
    //     println!("Please enter a valid address");
    //     return;
    // }

    // let ip_port = "127.0.0.1:8080";
    let ip_port = args.get(1).expect("Argument error");
    
    let sckt = tcp_connect(ip_port).expect("Error when creating TCP socket");
    // let data = ""; // TODO: Parameterize the data

    send_tcp_packet(sckt);
    
}

// TODO: Move ip parsing operations to another function
// TODO: Store the ring anywhere else (Mandatory)
// TODO: Move opcode creations into their own expressions
