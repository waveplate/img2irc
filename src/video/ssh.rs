//! Public SSH video service, with username-based selections and a fallback.
use super::*;
use russh::server::{Server as _, Session};
use russh::{server, Channel, ChannelId, ChannelOpenFailure, Pty};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::watch,
};

const ENTER: &[u8] = b"\x1b[?1049h\x1b[?6l\x1b[r\x1b[?25l\x1b[?7l";
const LEAVE: &[u8] = b"\x1b[?2026l\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l";

pub(super) fn bind_address(value: &str) -> std::result::Result<SocketAddr, String> {
    if let Ok(port) = value.parse::<u16>() {
        return Ok(SocketAddr::from(([0u16; 8], port)));
    }
    super::bind_address(value)
}

fn listener(address: SocketAddr) -> io::Result<tokio::net::TcpListener> {
    let socket = socket2::Socket::new(
        socket2::Domain::for_address(address),
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    if address.is_ipv6() && address.ip().is_unspecified() {
        socket.set_only_v6(false)?;
    }
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    socket.listen(128)?;
    tokio::net::TcpListener::from_std(socket.into())
}

fn host_key(path: &Path) -> Result<russh::keys::PrivateKey> {
    if path.exists() {
        return Ok(russh::keys::load_secret_key(path, None)?);
    }
    let key = russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)?;
    let encoded = key.to_openssh(russh::keys::ssh_key::LineEnding::LF)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(error) = file
        .write_all(encoded.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    eprintln!("Created SSH host key: {}", path.display());
    Ok(key)
}

pub(super) fn serve(input: PathBuf, bind: SocketAddr, key_path: PathBuf) -> Result<()> {
    let (header, _) = open_video(&input)?;
    let directory = Arc::new(std::env::current_dir()?);
    let key = host_key(&key_path)?;
    let config = Arc::new(server::Config {
        keys: vec![key],
        nodelay: true,
        channel_buffer_size: 4,
        event_buffer_size: 4,
        inactivity_timeout: Some(Duration::from_secs(60)),
        keepalive_interval: Some(Duration::from_secs(15)),
        ..Default::default()
    });
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let socket = listener(bind)?;
            eprintln!(
                "Serving {} ({}x{}, {} fps) over public SSH on {}",
                input.display(),
                header.width,
                header.height,
                header.fps,
                socket.local_addr()?
            );
            eprintln!("Username selections: {} (<username>.i2v); the supplied video is the fallback", directory.display());
            let mut server = VideoServer {
                input: Arc::new(input),
                directory,
            };
            server.run_on_socket(config, &socket).await
        })?;
    Ok(())
}

struct VideoServer {
    input: Arc<PathBuf>,
    directory: Arc<PathBuf>,
}

impl server::Server for VideoServer {
    type Handler = Viewer;

    fn new_client(&mut self, peer: Option<SocketAddr>) -> Viewer {
        let (size, _) = watch::channel((80, 24));
        let (quit, _) = watch::channel(false);
        Viewer {
            input: self.input.clone(),
            directory: self.directory.clone(),
            requested_video: None,
            peer,
            channel: None,
            channel_id: None,
            active: Arc::new(AtomicBool::new(false)),
            size,
            quit,
        }
    }

    fn handle_session_error(&mut self, error: russh::Error) {
        eprintln!("SSH session: {error}");
    }
}

struct Viewer {
    input: Arc<PathBuf>,
    directory: Arc<PathBuf>,
    requested_video: Option<PathBuf>,
    peer: Option<SocketAddr>,
    channel: Option<Channel<server::Msg>>,
    channel_id: Option<ChannelId>,
    active: Arc<AtomicBool>,
    size: watch::Sender<(u16, u16)>,
    quit: watch::Sender<bool>,
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.quit.send_replace(true);
    }
}

fn resize(previous: (u16, u16), width: u32, height: u32) -> (u16, u16) {
    // SSH uses u32 sizes. Ignore zero dimensions and bound them before fitting.
    let dimension = |old, new: u32| {
        if new == 0 {
            old
        } else {
            new.min(u16::MAX.into()) as u16
        }
    };
    (dimension(previous.0, width), dimension(previous.1, height))
}

fn username_video(directory: &Path, username: &str) -> Option<PathBuf> {
    // Usernames select a local basename, never a directory or arbitrary path.
    // Reject separators on both Unix and Windows, and names too long for a
    // conventional filesystem component after appending the extension.
    if username.is_empty()
        || matches!(username, "." | "..")
        || username.len() > 251
        || username
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
    {
        return None;
    }
    Some(directory.join(format!("{username}.i2v")))
}

fn refuse(session: &mut Session, channel: ChannelId) -> std::result::Result<(), russh::Error> {
    session.channel_failure(channel)?;
    session.exit_status_request(channel, 126)?;
    session.eof(channel)?;
    session.close(channel)
}

impl server::Handler for Viewer {
    type Error = russh::Error;

    async fn auth_none(
        &mut self,
        username: &str,
    ) -> std::result::Result<server::Auth, Self::Error> {
        self.requested_video = username_video(&self.directory, username);
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        if self.active.swap(true, Ordering::Relaxed) {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
        } else {
            self.channel_id = Some(channel.id());
            self.size = watch::channel((80, 24)).0;
            self.quit = watch::channel(false).0;
            self.channel = Some(channel);
            reply.accept().await;
        }
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        width: u32,
        height: u32,
        _pixel_width: u32,
        _pixel_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        if self.channel_id != Some(channel) {
            return session.channel_failure(channel);
        }
        self.size
            .send_modify(|size| *size = resize(*size, width, height));
        session.channel_success(channel)
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        width: u32,
        height: u32,
        _pixel_width: u32,
        _pixel_height: u32,
        session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        if self.channel_id != Some(channel) {
            return session.channel_failure(channel);
        }
        self.size
            .send_modify(|size| *size = resize(*size, width, height));
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        id: ChannelId,
        session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        let Some(channel) = self.channel.take().filter(|channel| channel.id() == id) else {
            return session.channel_failure(id);
        };
        session.channel_success(id)?;
        let input = self.input.clone();
        let requested_video = self.requested_video.clone();
        let peer = self.peer;
        let size = self.size.subscribe();
        let quit = self.quit.clone();
        let handle = session.handle();
        let active = self.active.clone();
        tokio::spawn(async move {
            let _active = ActivePlayback(active);
            if let Err(error) = playback(channel, handle, input, requested_video, size, quit).await
            {
                eprintln!("SSH viewer {peer:?}: {error}");
            }
        });
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _command: &[u8],
        session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        if self
            .channel
            .as_ref()
            .is_some_and(|pending| pending.id() == channel)
        {
            self.active.store(false, Ordering::Relaxed);
            refuse(session, channel)
        } else {
            session.channel_failure(channel)
        }
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        _subsystem: &str,
        session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        if self
            .channel
            .as_ref()
            .is_some_and(|pending| pending.id() == channel)
        {
            self.active.store(false, Ordering::Relaxed);
            refuse(session, channel)
        } else {
            session.channel_failure(channel)
        }
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        _name: &str,
        _value: &str,
        session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        session.channel_failure(channel)
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _single: bool,
        _protocol: &str,
        _cookie: &str,
        _screen: u32,
        session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        session.channel_failure(channel)
    }

    async fn agent_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> std::result::Result<bool, Self::Error> {
        session.channel_failure(channel)?;
        Ok(false)
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        if self.channel_id == Some(channel) {
            self.quit.send_replace(true);
            if self.channel.is_some() {
                self.channel = None;
                self.active.store(false, Ordering::Relaxed);
            }
        }
        Ok(())
    }

    async fn signal(
        &mut self,
        channel: ChannelId,
        signal: russh::Sig,
        _session: &mut Session,
    ) -> std::result::Result<(), Self::Error> {
        if self.channel_id == Some(channel)
            && matches!(signal, russh::Sig::INT | russh::Sig::TERM | russh::Sig::HUP)
        {
            self.quit.send_replace(true);
        }
        Ok(())
    }
}

struct AbortTask(tokio::task::JoinHandle<()>);
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct ActivePlayback(Arc<AtomicBool>);
impl Drop for ActivePlayback {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

async fn send(output: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), output.write_all(bytes)).await??;
    Ok(())
}

async fn packet(
    presenter: &mut present::Presenter,
    raw: &[u8],
    header: &Header,
    size: (u16, u16),
    clear: bool,
) -> io::Result<Vec<u8>> {
    let raw = raw.to_vec();
    let header = header.clone();
    let mut state = std::mem::take(presenter);
    let (state, result) = tokio::task::spawn_blocking(move || {
        let result = state
            .prepare(&raw, &header, Some(size), true, clear)
            .map_err(|error| io::Error::other(error.to_string()));
        (state, result)
    })
    .await?;
    *presenter = state;
    result
}

async fn stopped(quit: &mut watch::Receiver<bool>) {
    loop {
        if *quit.borrow_and_update() {
            return;
        }
        if quit.changed().await.is_err() {
            return;
        }
    }
}

async fn playback(
    channel: Channel<server::Msg>,
    handle: server::Handle,
    path: Arc<PathBuf>,
    requested_video: Option<PathBuf>,
    mut size: watch::Receiver<(u16, u16)>,
    quit_sender: watch::Sender<bool>,
) -> io::Result<()> {
    let id = channel.id();
    let (mut keyboard, mut output) = tokio::io::split(channel.into_stream());
    let mut quit = quit_sender.subscribe();
    let _keyboard_task = AbortTask(tokio::spawn(async move {
        let mut buffer = [0; 256];
        loop {
            match keyboard.read(&mut buffer).await {
                Ok(0) => break, // stdin EOF still permits playback (e.g. ssh -n).
                Ok(count) if buffer[..count].iter().any(|c| matches!(c, b'q' | 3 | 27)) => {
                    quit_sender.send_replace(true);
                    break;
                }
                Ok(_) => {}
                Err(_) => {
                    quit_sender.send_replace(true);
                    break;
                }
            }
        }
    }));
    send(&mut output, ENTER).await?;
    let result = tokio::select! {
        _ = stopped(&mut quit) => Ok(()),
        result = play_frames(&mut output, &path, requested_video, &mut size) => result,
    };
    // Close only after restoring the screen and reporting an SSH exit status.
    let cleanup = send(&mut output, LEAVE).await;
    let _ = handle
        .exit_status_request(id, u32::from(result.is_err()))
        .await;
    let _ = handle.eof(id).await;
    let _ = handle.close(id).await;
    result?;
    cleanup
}

async fn play_frames(
    output: &mut (impl AsyncWrite + Unpin),
    path: &Path,
    requested_video: Option<PathBuf>,
    size: &mut watch::Receiver<(u16, u16)>,
) -> io::Result<()> {
    let fallback = path.to_owned();
    let (header, mut reader) = tokio::task::spawn_blocking(move || {
        if let Some(requested) = requested_video.filter(|path| path.is_file()) {
            match open_video(&requested) {
                Ok(video) => return Ok(video),
                Err(error) => eprintln!("SSH video {requested:?}: {error}; using fallback"),
            }
        }
        open_video(&fallback).map_err(|error| io::Error::other(error.to_string()))
    })
    .await??;
    send(output, clear_screen(header.foreground_only)).await?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    tokio::task::spawn_blocking(move || loop {
        let frame = read_frame(&mut reader);
        let done = !matches!(frame, Ok(Some(_)));
        if sender.blocking_send(frame).is_err() || done {
            break;
        }
    });
    let mut buffered = VecDeque::new();
    let mut finished = false;
    for _ in 0..4 {
        match receiver.recv().await.transpose()?.flatten() {
            Some(frame) => buffered.push_back(frame),
            None => {
                finished = true;
                break;
            }
        }
    }
    let first = buffered
        .pop_front()
        .ok_or_else(|| io::Error::other("video contains no frames"))?;
    let mut previous_size = *size.borrow_and_update();
    let mut presenter = present::Presenter::default();
    let first_packet = packet(&mut presenter, &first, &header, previous_size, false).await?;
    let started = tokio::time::Instant::now();
    send(output, &first_packet).await?;
    let mut last = first;
    let mut index = 1;
    loop {
        let raw = if let Some(frame) = buffered.pop_front() {
            Some(frame)
        } else if finished {
            None
        } else {
            receiver.recv().await.transpose()?.flatten()
        };
        let deadline = started + Duration::from_secs_f64(index as f64 / header.fps);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                changed = size.changed() => {
                    if changed.is_err() { return Ok(()); }
                    let current_size = *size.borrow_and_update();
                    let resized = packet(&mut presenter, &last, &header, current_size, true).await?;
                    send(output, &resized).await?;
                    previous_size = current_size;
                }
            }
        }
        let Some(raw) = raw else {
            return Ok(());
        };
        // Complete independent frames permit skipping late source frames.
        if started.elapsed().as_secs_f64() < (index + 1) as f64 / header.fps {
            let current_size = *size.borrow_and_update();
            let prepared = packet(
                &mut presenter,
                &raw,
                &header,
                current_size,
                current_size != previous_size,
            )
            .await?;
            send(output, &prepared).await?;
            last = raw;
            previous_size = current_size;
        }
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_sizes_ignore_zero_and_bound_large_dimensions() {
        assert_eq!(resize((80, 24), 0, 0), (80, 24));
        assert_eq!(resize((80, 24), 180, 68), (180, 68));
        assert_eq!(resize((80, 24), u32::MAX, 0), (u16::MAX, 24));
    }

    #[test]
    fn ssh_port_shorthand_uses_the_dual_stack_listener() {
        assert_eq!(bind_address("2222").unwrap(), "[::]:2222".parse().unwrap());
        assert_eq!(
            bind_address("127.0.0.1:2222").unwrap(),
            "127.0.0.1:2222".parse().unwrap()
        );
        assert!(bind_address("65536").is_err());
    }

    #[test]
    fn host_key_is_private_and_survives_restarts() {
        let path = std::env::temp_dir().join(format!(
            "img2irc-ssh-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let first = host_key(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let second = host_key(&path).unwrap();
        assert_eq!(first.public_key(), second.public_key());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_file(path).unwrap();
    }
}
