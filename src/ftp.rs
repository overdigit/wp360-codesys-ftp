use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::prelude::*;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use suppaftp::FtpError;
use suppaftp::rustls;
use suppaftp::rustls::ClientConfig;
use suppaftp::{RustlsConnector, RustlsFtpStream};

#[derive(Deserialize, Debug)]
enum Command {
    NoOp,
    Upload {
        local: String,
        remote: Option<String>,
    },
    Download {
        local: Option<String>,
        remote: String,
    },
    Rename {
        remote: String,
        new_name: String,
    },
    Delete {
        remote: String,
    },
    GetFileSize {
        remote: String,
    },
    SetDirectory {
        remote: String,
    },
    GetDirectory,
    CreateDirectory {
        remote: String,
    },
    DeleteDirectory {
        remote: String,
    },
    Connect {
        hostname: String,
        port: u16,
        username: String,
        password: String,
        passive: bool,
        tls: bool,
    },
}

// TODO: serialize as u32
#[derive(Debug)]
#[repr(u32)]
enum FtpResultError {
    UnexpectedResponse(u32),
    TlsError,
    BadResponse,
    DataConnectionAlreadyOpen,
    LocalForbidden, // local path not in base folder
    InvalidLocalPath,
    InvalidRemoteUTF8,
    RemoteIsDirectory,
    AlreadyConnected,
    NotConnected,
    SyntaxError,
    HostUnreachable,
    NetworkUnreachable,
    InvalidAddress,
    UnimplementedError
}

impl Serialize for FtpResultError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        match self {
            FtpResultError::UnexpectedResponse(code) => serializer.serialize_u32(code + 1000),

            FtpResultError::TlsError => serializer.serialize_u32(10),
            FtpResultError::BadResponse => serializer.serialize_u32(20),
            FtpResultError::DataConnectionAlreadyOpen => serializer.serialize_u32(30),
            FtpResultError::LocalForbidden => serializer.serialize_u32(40),
            FtpResultError::InvalidLocalPath => serializer.serialize_u32(50),
            FtpResultError::InvalidRemoteUTF8 => serializer.serialize_u32(60),
            FtpResultError::RemoteIsDirectory => serializer.serialize_u32(70),
            FtpResultError::AlreadyConnected => serializer.serialize_u32(80),
            FtpResultError::NotConnected => serializer.serialize_u32(90),
            FtpResultError::SyntaxError => serializer.serialize_u32(100),
            FtpResultError::HostUnreachable => serializer.serialize_u32(110),
            FtpResultError::NetworkUnreachable => serializer.serialize_u32(120),
            FtpResultError::InvalidAddress => serializer.serialize_u32(130),

            FtpResultError::UnimplementedError => serializer.serialize_u32(10000),
        }
    }
}

#[derive(Serialize, Debug)]
enum FtpResult {
    FileSize(usize),
    Directory(String),
    Success,
    Error(FtpResultError),
}
impl From<FtpResultError> for FtpResult {
    fn from(e: FtpResultError) -> Self {
        FtpResult::Error(e)
    }
}

impl From<FtpError> for FtpResult {
    fn from(e: FtpError) -> Self {
        match e {
            FtpError::ConnectionError(e) => FtpResultError::from(e).into(),
            FtpError::UnexpectedResponse(r) => FtpResultError::UnexpectedResponse(r.status.code()).into(),
            FtpError::SecureError(_s) => FtpResultError::TlsError.into(),
            FtpError::BadResponse => FtpResultError::BadResponse.into(),
            FtpError::InvalidAddress(_e) => FtpResultError::InvalidAddress.into(),
            FtpError::DataConnectionAlreadyOpen => FtpResultError::DataConnectionAlreadyOpen.into(),
        }
    }
}

impl From<std::io::Error> for FtpResultError {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::InvalidFilename => FtpResultError::LocalForbidden,
            std::io::ErrorKind::InvalidInput => FtpResultError::InvalidLocalPath,
            std::io::ErrorKind::InvalidData => FtpResultError::InvalidRemoteUTF8,
            std::io::ErrorKind::HostUnreachable => FtpResultError::HostUnreachable,
            std::io::ErrorKind::NetworkUnreachable => FtpResultError::NetworkUnreachable,

            _ => {
                eprintln!("{:?}", error);
                FtpResultError::UnimplementedError // TODO: implement errors
            }
        }
    }
}

impl From<Result<(), FtpError>> for FtpResult {
    fn from(res: Result<(), FtpError>) -> Self {
        match res {
            Ok(()) => FtpResult::Success,
            Err(e) => e.into(),
        }
    }
}

fn fatal_error(mut stream: &UnixStream, res: FtpResult) {
    // Reasoning for ignoring these errors: we're quitting anyway, nothing more we can do
    let _ = serde_xml_rs::ser::to_writer(&mut stream, &res);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

pub fn client(mut stream: UnixStream) {
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let message: Command = match serde_xml_rs::de::from_reader::<Command, &mut UnixStream>(&mut stream) {
        Ok(m) => m,
        Err(_) => {
            fatal_error(&stream, FtpResult::Error(FtpResultError::SyntaxError));
            return;
        },
    };
    let Command::Connect {
        hostname,
        port,
        username,
        password,
        passive,
        tls,
    } = message
    else {
        fatal_error(&stream, FtpResult::Error(FtpResultError::NotConnected));
        return;
    };

    let mut ftp_stream = match RustlsFtpStream::connect(format!("{}:{}", hostname, port)) {
        Ok(t) => t,
        Err(e) => {
            println!("{:?}", e);
            fatal_error(&stream, FtpResult::from(e));
            return;
        },
    };

    ftp_stream.set_mode(if passive {
        suppaftp::types::Mode::Passive
    } else {
        suppaftp::types::Mode::Active
    });
    if tls {
        match ftp_stream.into_secure(RustlsConnector::from(Arc::new(config)), &hostname) {
            Ok(t) => ftp_stream = t,
            Err(e) => {
                fatal_error(&stream, e.into());
                return;
            }
        }
    }

    match ftp_stream.login(username, password) {
        Ok(()) => { },
        Err(e) => {
            println!("{:?}", e);
            fatal_error(&stream, FtpResult::from(e));
            return;
        },
    }
    match ftp_stream.transfer_type(suppaftp::types::FileType::Binary) {
        Ok(()) => { },
        Err(e) => {
            fatal_error(&stream, e.into());
            return;
        }
    }

    serde_xml_rs::ser::to_writer(&mut stream, &FtpResult::Success);
    loop {
        let cmd = match serde_xml_rs::de::from_reader(&mut stream) {
            Ok(t) => t,
            Err(e) => {
                println!("{:?}", e);
                fatal_error(&stream, FtpResult::Error(FtpResultError::SyntaxError));
                let _ = ftp_stream.quit(); // We're quitting either way, no need to handle err
                break;
            },
        };
        let res = perform_operation(&mut ftp_stream, &cmd);
        let Ok(()) = serde_xml_rs::ser::to_writer(&mut stream, &res) else {
            break;
        };
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn perform_operation(ftp: &mut RustlsFtpStream, cmd: &Command) -> FtpResult {
    match cmd {
        Command::NoOp => ftp.noop().into(),

        Command::Upload { local, remote } => {
            let path = match ftp_path(Path::new(local)) {
                Ok(t) => t,
                Err(e) => return e.into(),
            };
            let mut file = match File::open(&path) {
                Ok(t) => t,
                Err(e) => return FtpResultError::from(e).into(),
            };
            let filename_path = path.file_name().unwrap().to_str();
            let filename = match remote {
                Some(t) => t,
                None => match filename_path {
                    Some(p) => p,
                    None => return FtpResultError::InvalidRemoteUTF8.into(),
                },
            };
            match ftp.put_file(filename, &mut file) {
                Ok(_) => FtpResult::Success,
                Err(e) => e.into(),
            }
        }

        Command::Download { local, remote } => {
            if remote.ends_with("/") {
                return FtpResultError::RemoteIsDirectory.into();
            }
            let path = match local {
                Some(p) => match ftp_path(Path::new(p)) {
                    Ok(t) => t,
                    Err(e) => return e.into(),
                },
                None => match Path::new(remote).file_name().unwrap().to_str() {
                    Some(p) => match ftp_path(Path::new(p)) {
                        Ok(t) => t,
                        Err(e) => return e.into(),
                    },
                    None => return FtpResultError::InvalidRemoteUTF8.into(),
                },
            };
            let mut file = match File::create(&path) {
                Ok(t) => t,
                Err(e) => return FtpResultError::from(e).into(),
            };
            let buf = match ftp.retr_as_buffer(remote) {
                Ok(t) => t,
                Err(e) => return e.into(),
            };
            let Err(e) = file.write_all(buf.get_ref()) else {
                return FtpResult::Success;
            };
            FtpResultError::from(e).into()
        }

        Command::Rename { remote, new_name } => ftp.rename(remote, new_name).into(),

        Command::Delete { remote } => ftp.rm(remote).into(),

        Command::GetFileSize { remote } => match ftp.size(remote) {
            Ok(size) => FtpResult::FileSize(size),
            Err(e) => e.into(),
        },

        Command::SetDirectory { remote } => ftp.cwd(remote).into(),

        Command::GetDirectory => match ftp.pwd() {
            Ok(directory) => FtpResult::Directory(directory),
            Err(e) => e.into(),
        },

        Command::CreateDirectory { remote } => ftp.mkdir(remote).into(),

        Command::DeleteDirectory { remote } => ftp.rmdir(remote).into(),

        Command::Connect { .. } => FtpResultError::AlreadyConnected.into(),
    }
}

fn ftp_path(path: &Path) -> Result<PathBuf, FtpResultError> {
    let base_path = Path::new("/home/nix/build/wp360-codesys-ftp/");
    let complete_path = base_path.join(path);

    if complete_path == base_path {
        return Ok(complete_path);
    }

    let Some(parent) = complete_path.parent() else {
        return Err(FtpResultError::InvalidLocalPath);
    };
    let Some(filename) = complete_path.file_name() else {
        return Err(FtpResultError::InvalidLocalPath);
    };

    let true_parent = parent.canonicalize()?;
    if !true_parent.starts_with(base_path) {
        return Err(FtpResultError::LocalForbidden);
    }
    let true_path = parent.join(filename);
    Ok(true_path)
}
