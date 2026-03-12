use std::error::Error;
use std::thread;
use std::os::unix::net::UnixListener;

mod ftp;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    // TODO: remove socket if it exists

    let listener = UnixListener::bind("/var/opt/codesyscontrolapi/extfuncs/wp360-ftp.sock")?; // TODO: handle error, and fix the path

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(|| ftp::client(stream));
            }
            Err(err) => {
                break; // TODO: we probably don't _want_ to break? then again, depending on the error, we might need to
            }
        }
    }
    // TODO: remove socket
    Ok(())
}
