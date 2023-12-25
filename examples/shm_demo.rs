use std::{
    fs::File,
    io,
    mem,
    os::unix::io::AsRawFd,
    thread,
    time::Duration
};

fn from_syscall_error(error: syscall::Error) -> io::Error {
    io::Error::from_raw_os_error(error.errno as i32)
}
fn main() -> Result<(), io::Error> {
    let file = File::open("shm:counter")?;
    println!("Reading from map... ");
    let counter = unsafe {
        &mut *(syscall::fmap(file.as_raw_fd() as usize, &syscall::Map {
            offset: 0,
            address: 0,
            size: mem::size_of::<usize>(),
            flags: syscall::PROT_READ | syscall::PROT_WRITE | syscall::MAP_SHARED,
        }).map_err(from_syscall_error)? as *mut usize)
    };
    println!("Read value {}", counter);
    *counter += 1;
    println!("Increased value to {}", counter);

    thread::sleep(Duration::from_secs(1));
    Ok(())
}
