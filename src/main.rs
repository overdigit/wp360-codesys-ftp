use configparser::ini::Ini;
use inotify::{EventMask, Inotify, WatchMask};
use std::collections::HashMap;
use std::error::Error;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::OnceLock;
use std::thread;

static PLACEHOLDERS: OnceLock<HashMap<String, String>> = OnceLock::new();

mod ftp;

fn read_placeholders(config_path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let config = std::fs::read_to_string(config_path)?;
    let mut ini = Ini::new();
    let data = &ini.read(config)?;
    let Some(sysfile) = data.get("sysfile") else {
        let _ = PLACEHOLDERS.set(HashMap::new());
        return Ok(());
    };
    let placeholders: HashMap<String, String> = sysfile
        .iter()
        .filter(|(k, _v)| {
            k.strip_prefix("placeholderfilepath.")
                .and_then(|n| n.parse::<i32>().ok())
                .is_some()
        })
        .filter_map(|(_k, v)| {
            v.as_ref()
                .and_then(|v| v.rsplit_once(','))
                .map(|(v1, v2)| (v2.to_string(), v1.to_string()))
        })
        .collect();
    let _ = PLACEHOLDERS.set(placeholders);
    Ok(())
}

// Credit: https://github.com/nroi/waitforfile/blob/master/src/main.rs
fn wait_for(dirname: &std::path::Path) -> Result<bool, Box<dyn Error>> {
    let mut ino = Inotify::init()?;
    let parent = dirname.parent().expect("This folder has no parent. How? It's the default folder, I wrote its path. What did you do?");
    ino.watches()
        .add(parent, WatchMask::DELETE_SELF | WatchMask::CREATE)
        .unwrap();
    if !dirname.exists() {
        loop {
            let mut buffer = [0; 1024];
            let events = ino
                .read_events_blocking(&mut buffer)
                .expect("Error while reading events");
            for event in events {
                match event.name {
                    Some(name) => {
                        if parent.join(name) == dirname {
                            return Ok(true);
                        }
                    }
                    None => {
                        if event.mask == EventMask::DELETE_SELF {
                            return Err("The watched directory has been deleted.".into());
                        }
                    }
                }
            }
        }
    }
    // file already exists prior to running this program.
    Ok(false)
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let dir_env = std::env::var("WP360_CODESYS_SOCK_DIR");
    let env_set = dir_env.is_ok();
    let dir = dir_env.unwrap_or("/var/opt/codesyscontrolapi/extfuncs/".to_string());

    let dir_path = std::path::Path::new(&dir);
    if !dir_path.is_dir() {
        if env_set {
            return Err(
                format!("Invalid path - \"{dir}\" does not exist or is not a directory").into(),
            );
        }
        let path = std::path::Path::new(&dir);
        wait_for(path)?;
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

    let conf_env = std::env::var("WP360_CODESYS_USER_CFG");
    let conf = conf_env.unwrap_or("/etc/codesyscontrol/CODESYSControl_User.cfg".to_string());
    read_placeholders(std::path::Path::new(&conf))?;

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
