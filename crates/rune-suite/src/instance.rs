//! One window per app, and links handed to it.
//!
//! On Linux a link opens by starting the app again with the link as its argument. [`run`]
//! makes that second start hand the link (and the activation token that lets it take focus)
//! to the app already running, over a socket in `$XDG_RUNTIME_DIR/rune/`, and exit. The
//! running app gets the link in its `on_link` and comes to the front.
//!
//! Headless runs (`--scene`, `--snap` and Rune's other flags) skip all this.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use rune::{AppHandle, Tx};

/// What the app does with a link: parse it (`Link::parse`) and go there.
pub type OnLink<S> = fn(&mut S, &str, &mut Tx);

/// Runs `app` as the only instance of `name` (`"rune-mail"`), with any link it was started with
/// given to `on_link`. If it's already running, hands the link over and returns instead.
pub fn run<S: 'static>(name: &str, app: rune::App<S>, on_link: OnLink<S>) {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (links, rest): (Vec<String>, Vec<String>) = argv.into_iter().partition(|a| crate::links::Link::parse(a).is_some());
    let headless = rest.iter().any(|a| a.starts_with("--"));
    let token = std::env::var("XDG_ACTIVATION_TOKEN").or_else(|_| std::env::var("DESKTOP_STARTUP_ID")).unwrap_or_default();
    let app = app.args(rest);
    if !headless {
        match claim(name) {
            Claim::Other(stream) => {
                if hand_over(stream, &links, &token).is_ok() {
                    return;
                }
            }
            Claim::Ours(listener) => serve(listener, app.handle(), on_link),
            Claim::Unavailable => {}
        }
    }
    let handle = app.handle();
    for link in links {
        handle.update(move |s, tx| on_link(s, &link, tx));
    }
    app.run();
}

fn socket(name: &str) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("rune").join(format!("{name}.sock"))
}

enum Claim {
    /// Another instance is listening.
    Other(UnixStream),
    /// This is the first: links come here.
    Ours(UnixListener),
    /// No socket could be made; run alone.
    Unavailable,
}

fn claim(name: &str) -> Claim {
    let path = socket(name);
    if let Ok(stream) = UnixStream::connect(&path) {
        return Claim::Other(stream);
    }
    // Nobody answers: left from an instance that ended.
    let _ = std::fs::remove_file(&path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    match UnixListener::bind(&path) {
        Ok(l) => Claim::Ours(l),
        Err(_) => Claim::Unavailable,
    }
}

/// Sends `token <t>`, then `open <link>` for each link (or `show` with none).
fn hand_over(mut stream: UnixStream, links: &[String], token: &str) -> std::io::Result<()> {
    let mut msg = String::new();
    if !token.is_empty() {
        msg.push_str(&format!("token {}\n", one_line(token)));
    }
    if links.is_empty() {
        msg.push_str("show\n");
    }
    for l in links {
        msg.push_str(&format!("open {}\n", one_line(l)));
    }
    stream.write_all(msg.as_bytes())?;
    stream.flush()
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], "")
}

fn serve<S: 'static>(listener: UnixListener, handle: AppHandle<S>, on_link: OnLink<S>) {
    std::thread::Builder::new()
        .name("rune-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut token = String::new();
                for line in BufReader::new(stream).lines().map_while(Result::ok).take(64) {
                    if let Some(t) = line.strip_prefix("token ") {
                        token = t.to_owned();
                    } else if let Some(link) = line.strip_prefix("open ") {
                        let link = link.to_owned();
                        handle.update(move |s, tx| on_link(s, &link, tx));
                    }
                }
                handle.activate(token);
            }
        })
        .expect("a thread for the instance socket");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_start_finds_the_first() {
        let name = format!("rune-suite-test-{}", std::process::id());
        let Claim::Ours(listener) = claim(&name) else { panic!("the first claims the socket") };
        let Claim::Other(stream) = claim(&name) else { panic!("the second finds it") };
        hand_over(stream, &["tel:123".into()], "tok").unwrap();
        let (got, _) = listener.accept().unwrap();
        let lines: Vec<String> = BufReader::new(got).lines().map_while(Result::ok).collect();
        assert_eq!(lines, ["token tok", "open tel:123"]);
        drop(listener);
        let _ = std::fs::remove_file(socket(&name));
    }
}
