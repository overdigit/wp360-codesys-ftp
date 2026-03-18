use serde::{Serialize, Deserialize};
use serde_xml_rs::{from_str, to_string};
use std::borrow::{Borrow, Cow};
use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::os::unix::net::UnixStream;
use std::collections::HashMap;
use suppaftp::FtpError;
use suppaftp::rustls;
use suppaftp::rustls::ClientConfig;
use suppaftp::{RustlsConnector, RustlsFtpStream};

#[derive(Deserialize, Debug)]
#[serde(tag = "command")]
enum Command<'a> {
    NoOp,
    Upload {
        local: Cow<'a, Path>,
        remote: Option<&'a str>,
    },
    Download {
        local: Option<&'a Path>,
        remote: &'a str,
    },
    Rename {
        remote: &'a str,
        new_name: &'a str,
    },
    Delete {
        remote: &'a str,
    },
    GetFileSize {
        remote: &'a str,
    },
    SetDirectory {
        remote: &'a str,
    },
    GetDirectory,
    CreateDirectory {
        remote: &'a str,
    },
    DeleteDirectory {
        remote: &'a str,
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
#[derive(Serialize, Debug)]
#[repr(u32)]
enum FtpResultError {
    UnexpectedResponse(u32),
    TlsError,
    BadResponse,
    DataConnectionAlreadyOpen,
    LocalForbidden, // local path not in base folder
    InvalidPath,
    InvalidRemoteUTF8,
    AlreadyConnected,
}

#[derive(Serialize, Debug)]
enum FtpResult {
    FileSize { size: usize },
    Directory { directory: String },
    Success { },
    Error(FtpResultError),
}
impl From<FtpResultError> for FtpResult {
    fn from(e: FtpResultError) -> Self {
        FtpResult::Error(e)
    }
}

impl From<OperationError> for FtpResult {
    fn from(e: OperationError) -> Self {
        match e {
            OperationError::Ftp(e) => match e {
                FtpError::ConnectionError(e) => e.into(),
                FtpError::UnexpectedResponse(r) => FtpResultError::UnexpectedResponse(r.status.code()).into(),
                FtpError::SecureError(s) => FtpResultError::TlsError.into(),
                FtpError::BadResponse => FtpResultError::BadResponse.into(),
                FtpError::InvalidAddress(_e) => {
                    unreachable!()
                }
                FtpError::DataConnectionAlreadyOpen => FtpResultError::DataConnectionAlreadyOpen.into(),
            },
            OperationError::Io(e) => e.into(),
        }
    }
}

impl From<std::io::Error> for FtpResult {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::InvalidFilename => FtpResultError::LocalForbidden.into(),
            std::io::ErrorKind::InvalidInput => FtpResultError::InvalidPath.into(), 
            std::io::ErrorKind::InvalidData => FtpResultError::InvalidRemoteUTF8.into(),
            _ => {
                eprintln!("{:?}", error);
                todo!()
            }
        }
    }
}


#[derive(Debug)]
enum OperationError {
    Ftp(FtpError),
    Io(std::io::Error),
}
impl From<std::io::Error> for OperationError {
    fn from(error: std::io::Error) -> Self {
        OperationError::Io(error)
    }
}
impl From<FtpError> for OperationError {
    fn from(error: FtpError) -> Self {
        OperationError::Ftp(error)
    }
}

pub fn client(mut stream: UnixStream) {
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let Ok(message) = serde_xml_rs::de::from_reader(&mut stream) else {
        // TODO: send error back
        stream.shutdown(std::net::Shutdown::Both);
        return
    };
    let Command::Connect { hostname, port, username, password, passive, tls } = message else {
        // TODO: send error, must be Connect
        todo!();
    };

    let Ok(mut ftp_stream) = RustlsFtpStream::connect(format!(
        "{}:{}",
        hostname, port
    )) else { todo!(); }; // TODO: return some error code

    ftp_stream.set_mode(if passive {
        suppaftp::types::Mode::Passive
    } else {
        suppaftp::types::Mode::Active
    });
    if tls {
        match ftp_stream.into_secure(
            RustlsConnector::from(Arc::new(config)),
            &hostname,
        ) {
            Ok(t) => ftp_stream = t,
            Err(e) => todo!(),
        }
    }

    ftp_stream.login(username, password).unwrap(); // TODO: this will not do
    ftp_stream.transfer_type(suppaftp::types::FileType::Binary).unwrap(); // TODO: this will not do

    loop {
        let cmd;
        match serde_xml_rs::de::from_reader(&mut stream) {
            Ok(t) => cmd = t,
            Err(e) => todo!(),
        }
        match perform_operation(&mut ftp_stream, &cmd) {
            Ok(t) => todo!(),
            Err(e) => todo!(),
        }
    }
    stream.shutdown(std::net::Shutdown::Both);
    ()
}

fn perform_operation(
    ftp: &mut RustlsFtpStream,
    cmd: &Command,
) -> Result<FtpResult, OperationError> { // TODO: OperationError can probably go, should just be FtpResult
    match cmd {
        Command::NoOp {} => {
            ftp.noop()?;
        }

        Command::Upload { local, remote } => {
            let path = ftp_path(local.borrow())?;
            let mut file = File::open(&path)?;
            let filename;
            match remote {
                Some(t) => filename = t,
                None => match &path.file_name().unwrap().to_str() {
                    Some(p) => filename = p,
                    None => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Invalid UTF8 sequence in remote filename",
                        )
                        .into());
                    }
                },
            }
            ftp.put_file(remote.unwrap_or(filename), &mut file)?;
        }

        Command::Download { local, remote } => {
            let path;
            if remote.ends_with("/") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::IsADirectory,
                    "Remote path is a directory",
                )
                .into());
            }
            match local {
                Some(p) => path = ftp_path(p)?,
                None => match Path::new(remote).file_name().unwrap().to_str() {
                    Some(p) => path = ftp_path(Path::new(p))?,
                    None => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Invalid UTF8 sequence in remote filename",
                        )
                        .into());
                    }
                },
            }
            let mut file = File::create(&path)?;
            let buf = ftp.retr_as_buffer(remote)?;
            file.write_all(buf.get_ref())?;
        }

        Command::Rename { remote, new_name } => {
            ftp.rename(remote, new_name)?;
        }

        Command::Delete { remote } => {
            ftp.rm(remote)?;
        }

        Command::GetFileSize { remote } => {
            let size = ftp.size(remote)?;
            return Ok(FtpResult::FileSize { size });
        }

        Command::SetDirectory { remote } => {
            ftp.cwd(remote)?;
        }

        Command::GetDirectory {} => {
            let path = ftp.pwd()?;
            return Ok(FtpResult::Directory { directory: path });
        }

        Command::CreateDirectory { remote } => {
            ftp.mkdir(remote)?;
        }

        Command::DeleteDirectory { remote } => {
            ftp.rmdir(remote)?;
        }

        Command::Connect { .. } => {
            return Err(FtpResult::Error(FtpResultError::AlreadyConnected).into());
        }

    }
    Ok(FtpResult::Success { })
}

fn ftp_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    let base_path = Path::new("/home/nix/build/wp360-codesys-bridge-rs/");
    let complete_path = base_path.join(path);

    if complete_path == base_path {
        return Ok(complete_path);
    }

    let Some(parent) = complete_path.parent() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid local path",
        ));
    };
    let Some(filename) = complete_path.file_name() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid local path",
        ));
    };

    let true_parent = parent.canonicalize()?;
    if !true_parent.starts_with(base_path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidFilename,
            "Local path not in base folder",
        ));
    }
    let true_path = parent.join(filename);
    Ok(true_path)
}
