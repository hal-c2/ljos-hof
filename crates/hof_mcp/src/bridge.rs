//! `hof mcp`: the stdio end agents start, joined to the app's socket.

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;

/// Join stdin and stdout to the server at `socket_path` until either side
/// hangs up. This is what `hof mcp` runs, so an agent configured with
/// `{"command": "hof", "args": ["mcp"]}` talks to the running Hof.
pub fn bridge_stdio(socket_path: impl AsRef<Path>) -> io::Result<()> {
    bridge(socket_path, io::stdin(), io::stdout())
}

/// [`bridge_stdio`] with any input and output.
///
/// Returns once the server hangs up, which it does after answering
/// everything sent before `input` ended.
pub fn bridge<R, W>(socket_path: impl AsRef<Path>, input: R, mut output: W) -> io::Result<()>
where
    R: Read + Send + 'static,
    W: Write,
{
    let socket_path = socket_path.as_ref();
    let socket = UnixStream::connect(socket_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot reach Hof's MCP server at {}: {error}; is Hof running with MCP enabled?",
                socket_path.display()
            ),
        )
    })?;
    let mut to_hof = socket.try_clone()?;
    // Not joined: when Hof goes away first this thread is still blocked
    // reading stdin, and the process exiting is what ends it.
    thread::Builder::new()
        .name("hof-mcp-stdin".into())
        .spawn(move || {
            let mut input = input;
            let _ = io::copy(&mut input, &mut to_hof);
            // Lets the server see the end of input and hang up once it
            // has answered.
            let _ = to_hof.shutdown(Shutdown::Write);
        })?;

    let mut from_hof = &socket;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = match from_hof.read(&mut buffer) {
            Ok(0) => return output.flush(),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        output.write_all(&buffer[..read])?;
        // The agent is waiting on this reply; it cannot sit in a buffer.
        output.flush()?;
    }
}
