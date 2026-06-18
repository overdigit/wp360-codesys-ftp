use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::prelude::*;
use std::net::ToSocketAddrs;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use suppaftp::FtpError;
use suppaftp::rustls;
use suppaftp::rustls::ClientConfig;
use suppaftp::{RustlsConnector, RustlsFtpStream};

use crate::PLACEHOLDERS;

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
        timeout: Option<u64>,
    },
}

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
    UnimplementedError,
    NetworkDown,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    TimedOut,
    AlreadyExists,
    IsADirectory,
    ReadOnlyFilesystem,
    StorageFull,
    QuotaExceeded,
    FileTooLarge,
    FileNotFound,
    IOOther,
}
const FTP_ERROR_CODE_BASE: u32 = 0;
impl Serialize for FtpResultError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        match self {
            // 100 - 1000
            FtpResultError::UnexpectedResponse(code) => {
                serializer.serialize_u32(FTP_ERROR_CODE_BASE + code)
            }

            FtpResultError::NetworkDown => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 10),
            FtpResultError::NetworkUnreachable => {
                serializer.serialize_u32(FTP_ERROR_CODE_BASE + 12)
            }
            FtpResultError::HostUnreachable => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 14),
            FtpResultError::ConnectionRefused => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 16),
            FtpResultError::TlsError => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 18),
            FtpResultError::ConnectionReset => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 20),
            FtpResultError::ConnectionAborted => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 22),
            FtpResultError::TimedOut => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 24),

            FtpResultError::SyntaxError => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 30),
            FtpResultError::NotConnected => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 32),
            FtpResultError::InvalidAddress => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 34),
            FtpResultError::AlreadyConnected => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 36),
            FtpResultError::InvalidRemoteUTF8 => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 38),
            FtpResultError::DataConnectionAlreadyOpen => {
                serializer.serialize_u32(FTP_ERROR_CODE_BASE + 40)
            }
            FtpResultError::BadResponse => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 42),

            FtpResultError::LocalForbidden => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 50),
            FtpResultError::InvalidLocalPath => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 52),
            FtpResultError::RemoteIsDirectory => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 54),
            FtpResultError::AlreadyExists => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 56),
            FtpResultError::IsADirectory => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 58),
            FtpResultError::ReadOnlyFilesystem => {
                serializer.serialize_u32(FTP_ERROR_CODE_BASE + 60)
            }
            FtpResultError::StorageFull => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 62),
            FtpResultError::QuotaExceeded => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 64),
            FtpResultError::FileTooLarge => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 66),
            FtpResultError::FileNotFound => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 68),

            FtpResultError::IOOther => serializer.serialize_u32(FTP_ERROR_CODE_BASE + 90),
            FtpResultError::UnimplementedError => {
                serializer.serialize_u32(FTP_ERROR_CODE_BASE + 91)
            }
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(rename = "Result")]
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

impl From<FtpError> for FtpResultError {
    fn from(e: FtpError) -> Self {
        match e {
            FtpError::ConnectionError(e) => FtpResultError::from(e),
            FtpError::UnexpectedResponse(r) => FtpResultError::UnexpectedResponse(r.status.code()),
            FtpError::SecureError(_s) => FtpResultError::TlsError,
            FtpError::BadResponse => FtpResultError::BadResponse,
            FtpError::InvalidAddress(_e) => FtpResultError::InvalidAddress,
            FtpError::DataConnectionAlreadyOpen => FtpResultError::DataConnectionAlreadyOpen,
        }
    }
}

impl From<std::io::Error> for FtpResultError {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::HostUnreachable => FtpResultError::HostUnreachable,
            std::io::ErrorKind::NetworkUnreachable => FtpResultError::NetworkUnreachable,

            std::io::ErrorKind::PermissionDenied => FtpResultError::LocalForbidden, // TODO: is this correct? Probably, but worth thinking over
            std::io::ErrorKind::ConnectionRefused => FtpResultError::ConnectionRefused,
            std::io::ErrorKind::ConnectionReset => FtpResultError::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted => FtpResultError::ConnectionAborted,
            std::io::ErrorKind::NetworkDown => FtpResultError::NetworkDown,
            std::io::ErrorKind::AlreadyExists => FtpResultError::AlreadyExists, // Probably won't happen, as we happily overwrite?
            std::io::ErrorKind::IsADirectory => FtpResultError::IsADirectory,
            std::io::ErrorKind::ReadOnlyFilesystem => FtpResultError::ReadOnlyFilesystem, // Probably shouldn't happen unless they try writing on an iso usb drive
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                FtpResultError::TimedOut
            }
            std::io::ErrorKind::StorageFull => FtpResultError::StorageFull,
            std::io::ErrorKind::QuotaExceeded => FtpResultError::QuotaExceeded,
            std::io::ErrorKind::FileTooLarge => FtpResultError::FileTooLarge,
            std::io::ErrorKind::Other => FtpResultError::IOOther,
            std::io::ErrorKind::NotFound => FtpResultError::FileNotFound,

            _ => {
                FtpResultError::UnimplementedError // TODO: implement errors
            }
        }
    }
}

impl From<Result<(), FtpError>> for FtpResult {
    fn from(res: Result<(), FtpError>) -> Self {
        match res {
            Ok(()) => FtpResult::Success,
            Err(e) => FtpResultError::from(e).into(),
        }
    }
}

fn fatal_error(mut stream: &UnixStream, res: &FtpResult) {
    // Reasoning for ignoring these errors: we're quitting anyway, nothing more we can do
    let _ = serde_xml_rs::ser::to_writer(&mut stream, &res);
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

pub fn client(mut stream: UnixStream) {
    let root_store = webpki_roots::TLS_SERVER_ROOTS
        .iter()
        .cloned()
        .collect::<rustls::RootCertStore>();

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let Ok(message) = serde_xml_rs::de::from_reader::<Command, &mut UnixStream>(&mut stream) else {
        fatal_error(&stream, &FtpResult::Error(FtpResultError::SyntaxError));
        return;
    };

    let Command::Connect {
        hostname,
        port,
        username,
        password,
        passive,
        tls,
        timeout,
    } = message
    else {
        fatal_error(&stream, &FtpResult::Error(FtpResultError::NotConnected));
        return;
    };
    let Ok(Some(addr)) = format!("{hostname}:{port}").to_socket_addrs().map(|mut i| i.next()) else {
        fatal_error(&stream, &FtpResult::Error(FtpResultError::InvalidAddress));
        return;
    };

    let connection_r = if let Some(t) = timeout
        && t > 0
    {
        RustlsFtpStream::connect_timeout(addr, std::time::Duration::from_millis(t))
    } else {
        RustlsFtpStream::connect(addr)
    };
    let mut ftp_stream = match connection_r {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e:?}");
            fatal_error(&stream, &FtpResultError::from(e).into());
            return;
        }
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
                fatal_error(&stream, &FtpResultError::from(e).into());
                return;
            }
        }
    }

    if let Err(e) = ftp_stream
        .get_ref()
        .set_read_timeout(timeout.map(std::time::Duration::from_millis))
    {
        fatal_error(&stream, &FtpResultError::from(e).into());
        return;
    }

    if let Err(e) = ftp_stream
        .get_ref()
        .set_write_timeout(timeout.map(std::time::Duration::from_millis))
    {
        fatal_error(&stream, &FtpResultError::from(e).into());
        return;
    }

    if let Err(e) = ftp_stream.login(username, password) {
        eprintln!("{e:?}");
        fatal_error(&stream, &FtpResultError::from(e).into());
        return;
    }
    if let Err(e) = ftp_stream.transfer_type(suppaftp::types::FileType::Binary) {
        fatal_error(&stream, &FtpResultError::from(e).into());
        return;
    }

    if let Err(_e) = serde_xml_rs::ser::to_writer(&mut stream, &FtpResult::Success) {
        let _ = ftp_stream.quit(); // No need to handle the error; we already can't communicate back
        return;
    }

    loop {
        let cmd = match serde_xml_rs::de::from_reader(&mut stream) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{e:?}");
                fatal_error(&stream, &FtpResult::Error(FtpResultError::SyntaxError));
                let _ = ftp_stream.quit(); // We're quitting either way, no need to handle err
                break;
            }
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
                Err(e) => {
                    eprintln!("{e:?}");
                    return e.into();
                }
            };
            let mut file = match File::open(&path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{e:?}");
                    return FtpResultError::from(e).into();
                }
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
                Err(e) => {
                    eprintln!("{e:?}");
                    FtpResultError::from(e).into()
                }
            }
        }

        Command::Download { local, remote } => {
            if remote.ends_with('/') {
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
            match ftp.retr(remote, move |reader| {
                std::io::copy(reader, &mut file)
                    .map(|_| ())
                    .map_err(FtpError::ConnectionError)
            }) {
                Ok(()) => FtpResult::Success,
                Err(e) => FtpResultError::from(e).into(),
            }
        }

        Command::Rename { remote, new_name } => ftp.rename(remote, new_name).into(),

        Command::Delete { remote } => ftp.rm(remote).into(),

        Command::GetFileSize { remote } => match ftp.size(remote) {
            Ok(size) => FtpResult::FileSize(size),
            Err(e) => FtpResultError::from(e).into(),
        },

        Command::SetDirectory { remote } => ftp.cwd(remote).into(),

        Command::GetDirectory => match ftp.pwd() {
            Ok(directory) => FtpResult::Directory(directory),
            Err(e) => FtpResultError::from(e).into(),
        },

        Command::CreateDirectory { remote } => ftp.mkdir(remote).into(),

        Command::DeleteDirectory { remote } => ftp.rmdir(remote).into(),

        Command::Connect { .. } => FtpResultError::AlreadyConnected.into(),
    }
}

fn sub_path_placeholders(path: &Path, paths: &HashMap<&String, &Path>) -> Result<PathBuf, FtpResultError> {
    let Some(first) = path.components().next() else {
        return Err(FtpResultError::InvalidLocalPath);
    };

    if let std::path::Component::Normal(f) = first 
        && let Some((_placeholder_id, placeholder_path)) = paths.iter().find(|(k, _v)| ****k == *f) { // TODO: fix this mess
            let stripped_path = path.strip_prefix(first).unwrap();
            let new_path = placeholder_path.join(stripped_path);
            return Ok(new_path.clone());
        
    }
    Ok(path.to_owned())
}

fn ftp_path(path: &Path) -> Result<PathBuf, FtpResultError> {
    let base_path = Path::new("/var/opt/codesys/PlcLogic/");
    let mut paths = PLACEHOLDERS.get().unwrap().iter().map(|(k, v)| (k, Path::new(v))).collect::<HashMap<_,_>>();
    let empty_string = &String::new();
    paths.insert(empty_string, base_path);

    let buf = sub_path_placeholders(path, &paths)?;
    let path = &buf;

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
    if !paths.values().any(|p| true_parent.starts_with(p)) {
        return Err(FtpResultError::LocalForbidden);
    }
    let true_path = parent.join(filename);
    Ok(true_path)
}
