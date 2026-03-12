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
}

impl From<&CodesysMessage> for Command<'_> {
    fn from(m: &CodesysMessage) -> Self {
        let Some((_, method)) = m.params.get("method") else {panic!("No method in call")};
        match method.as_str() {
            "noop" => {
                return Command::NoOp;
            },
            "download" => {
                todo!();
            },
            "rename" => {
                todo!();
            },
            "delete" => {
                todo!();
            },
            "getfilesize" => {
                todo!();
            },
            "setdirectory" => {
                todo!();
            },
            "getdirectory" => {
                return Command::GetDirectory;
            },
            "createdirectory" => {
                todo!();
            },
            "deletedirectory" => {
                todo!();
            },
            _ => {
                panic!("Invalid method in call");
            }
        }
    }
}

enum FtpResult {
    GetFileSize {
        size: usize,
    },
    GetDirectory {
        path: String,
    },
    Generic {
        success: bool,
    },
    Error {
        success: bool,
        error: String,
        code: u32,
    },
}

impl From<OperationError> for FtpResult {
    fn from(e: OperationError) -> Self {
        match e {
            OperationError::Ftp(e) => match e {
                FtpError::ConnectionError(e) => e.into(),
                FtpError::UnexpectedResponse(r) => FtpResult::Error {
                    success: false,
                    code: 1000 + r.status.code(),
                    error: "Unexpected server response".to_string(),
                },
                FtpError::SecureError(s) => FtpResult::Error {
                    success: false,
                    code: 1001,
                    error: s,
                },
                FtpError::BadResponse => FtpResult::Error {
                    success: false,
                    code: 1002,
                    error: "Bad response".to_string(),
                },
                FtpError::InvalidAddress(_e) => {
                    unreachable!()
                }
                FtpError::DataConnectionAlreadyOpen => FtpResult::Error {
                    success: false,
                    code: 1003,
                    error: "Data connection already open".to_string(),
                },
            },
            OperationError::Io(e) => e.into(),
        }
    }
}

impl From<std::io::Error> for FtpResult {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::InvalidFilename => FtpResult::Error {
                success: false,
                code: 1004,
                error: "Local path not in base folder".to_string(),
            },
            std::io::ErrorKind::InvalidInput => FtpResult::Error {
                success: false,
                code: 1005,
                error: "Invalid local path".to_string(),
            },
            std::io::ErrorKind::InvalidData => FtpResult::Error {
                success: false,
                code: 1006,
                error: "Invalid UTF-8 in remote path".to_string(),
            },
            _ => {
                eprintln!("{:?}", error);
                todo!()
            }
        }
    }
}

#[derive(Debug)]
struct Connection {
    hostname: String,
    port: u16,
    username: String,
    password: String,
    passive: bool,
    tls: bool,
}
impl Default for Connection {
    fn default() -> Self {
        Connection {
            hostname: "".to_string(),
            port: 21,
            username: "".to_string(),
            password: "".to_string(),
            passive: true,
            tls: false,
        }
    }
}

enum ConnectionParseError {
    NoMethod,
    InvalidMethod,
    NoHostname,
}

fn get_connection_params(m: CodesysMessage) -> Result<Connection, ConnectionParseError> {
    let mut conn: Connection = Default::default();
    let Some((_, method)) = m.params.get("method") else {return Err(ConnectionParseError::NoMethod)};
    if method != "connect" {return Err(ConnectionParseError::InvalidMethod)};
    let Some((_, hostname)) = m.params.get("hostname") else {return Err(ConnectionParseError::NoHostname)};
    conn.hostname = hostname.to_string();
    if let Some((_, port)) = m.params.get("port") {
        if let Ok(port_n) = port.parse::<u16>() {
            conn.port = port_n;
        }
    }
    if let Some((_, username)) = m.params.get("username") {
        conn.username = username.to_string();
    }
    if let Some((_, password)) = m.params.get("password") {
        conn.password = password.to_string();
    }
    if let Some((_, passive)) = m.params.get("passive") {
        if let Ok(passive_b) = passive.parse::<bool>() {
            conn.passive = passive_b;
        }
    }
    if let Some((_, tls)) = m.params.get("tls") {
        if let Ok(tls_b) = tls.parse::<bool>() {
            conn.tls = tls_b;
        }
    }

    Ok(conn)
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

enum CodesysMessageKind {
    Call,
    Other,
}
impl From<u32> for CodesysMessageKind {
    fn from(i: u32) -> CodesysMessageKind {
        match i {
            0 => CodesysMessageKind::Call,
            _ => CodesysMessageKind::Other,
        }
    }
}

struct CodesysMessage {
    id: u32,
    kind: CodesysMessageKind,
    length: usize,
    params: HashMap<String, (String, String)>,
}
enum CodesysMessageError {
    ShortHeader,
    ShortBody,
    NoWalrus,
    NoType,
}

fn get_codesys_message(mut stream: &UnixStream) -> Result<CodesysMessage, CodesysMessageError> {
    let mut header = [0; 12];
    let header_len = stream.read(&mut header).expect("header read failed"); // TODO: handle error
    if header_len < 12 {
        return Err(CodesysMessageError::ShortHeader);
    }
    let (id_bytes, mut header) = header.split_at(size_of::<u32>());
    let id  = u32::from_ne_bytes(id_bytes.try_into().unwrap());
    let (kind_bytes, mut header) = header.split_at(size_of::<u32>());
    let kind = u32::from_ne_bytes(kind_bytes.try_into().unwrap()).into();
    let len = usize::from_ne_bytes(header.try_into().unwrap());
    let mut buf = vec![0; len];
    let buf_len = stream.read(&mut buf).expect("body read failed");
    if buf_len < len {
        return Err(CodesysMessageError::ShortBody); // TODO: peek and wait for a full message
    }
    let buf_str = String::from_utf8(buf).expect("invalid utf8");
    let params = buf_str.split('\0').flat_map(|part| {
        let Some((name, rest)) = part.split_once(":=") else {
            return Err(CodesysMessageError::NoWalrus);
        };
        let Some((type_, value)) = rest.split_once("#") else {
            return Err(CodesysMessageError::NoType);
        };
        Ok((name.to_string(), (type_.to_string(), value.to_string())))
    }).collect();
    Ok(CodesysMessage {
        id: id,
        kind: kind,
        length: len,
        params: params,
    })
}

pub fn client(stream: UnixStream) {
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let Ok(connection_message) = get_codesys_message(&stream) else {
        stream.shutdown(std::net::Shutdown::Both);
        return
    };

    let connection_params;
    match get_connection_params(connection_message) {
        Ok(t) => { connection_params = t;},
        Err(e) => { todo!();},
    }

    let Ok(mut ftp_stream) = RustlsFtpStream::connect(format!(
        "{}:{}",
        connection_params.hostname, connection_params.port
    )) else { todo!(); }; // TODO: return some error code

    ftp_stream.set_mode(if connection_params.passive {
        suppaftp::types::Mode::Passive
    } else {
        suppaftp::types::Mode::Active
    });
    if connection_params.tls {
        match ftp_stream.into_secure(
            RustlsConnector::from(Arc::new(config)),
            &connection_params.hostname,
        ) {
            Ok(t) => ftp_stream = t,
            Err(e) => todo!(),
        }
    }

    ftp_stream.login(connection_params.username, connection_params.password).unwrap(); // TODO: this will not do
    ftp_stream.transfer_type(suppaftp::types::FileType::Binary).unwrap(); // TODO: this will not do

    loop {
        let message;
        match get_codesys_message(&stream) {
            Ok(t) => message = t,
            Err(e) => todo!(),
        }
        match perform_operation(&mut ftp_stream, &message) {
            Ok(t) => todo!(),
            Err(e) => todo!(),
        }
    }
    stream.shutdown(std::net::Shutdown::Both);
    ()
}

fn perform_operation(
    ftp: &mut RustlsFtpStream,
    msg: &CodesysMessage,
) -> Result<FtpResult, OperationError> {
    let command: Command = msg.into();
    match command {
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
            return Ok(FtpResult::GetFileSize { size });
        }

        Command::SetDirectory { remote } => {
            ftp.cwd(remote)?;
        }

        Command::GetDirectory {} => {
            let path = ftp.pwd()?;
            return Ok(FtpResult::GetDirectory { path });
        }

        Command::CreateDirectory { remote } => {
            ftp.mkdir(remote)?;
        }

        Command::DeleteDirectory { remote } => {
            ftp.rmdir(remote)?;
        }
    }
    Ok(FtpResult::Generic { success: true })
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
