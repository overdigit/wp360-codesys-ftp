use std::error::Error;
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;

mod ftp;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let dir = std::env::var("WP360_CODESYS_SOCK_DIR")
        .unwrap_or("/var/opt/codesyscontrolapi/extfuncs/".to_string());

    let dir_path = std::path::Path::new(&dir);
    if !dir_path.is_dir() {
        return Err(format!(
            "Invalid path - \"{}\" does not exist or is not a directory",
            dir
        )
        .into());
    }
    let sock_path = dir_path.join("wp360-ftp.sock");
    match sock_path.try_exists() {
        Ok(true) => {
            match UnixStream::connect(&sock_path) {
                Ok(stream) => {
                    // Socket is in use
                    stream.shutdown(std::net::Shutdown::Both)?;
                    // Gonna let the ::bind call throw an error instead
                }
                Err(_e) => {
                    // Socket is not in use or at the very least not connectable, safe to remove
                    std::fs::remove_file(&sock_path)?;
                }
            }
        }
        Ok(false) => {}
        Err(e) => return Err(Box::new(e)),
    }

    let listener = UnixListener::bind(&sock_path)?;

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(|| ftp::client(stream));
            }
            Err(err) => {
                std::fs::remove_file(&sock_path)?;
                return Err(Box::new(err));
            }
        }
    }
    Ok(())
}
