# io_uring_scanner

Did not release an executable binary version since the batch and time values are not optimized yet I suppose, those will be added as command line arguments to adjust the trade of speed and reliability.

Quick note for usage (Need Cargo to build for now)

Build:

In directory, build with
```cargo build --release```

The binary ```io_uring_scanner``` will be generated inside ```./target/release```

Then run the binary
```./io_uring_scanner IP[/SUBNET_LENGTH] [OPTIONS]```
