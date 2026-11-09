//! Mosquitto 2 in a container, started by the test and removed when it is
//! dropped. Its ports are published on 127.0.0.1 at ports docker picks.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::pki::Pki;

pub(crate) const IMAGE: &str = "eclipse-mosquitto:2";
/// The dynamic-security administrator of `Mosquitto::with_dynamic_security`.
const ADMIN: &str = "admin";
const ADMIN_PASSWORD: &str = "admin-password";

pub(crate) struct Mosquitto {
    id: String,
    /// The configuration directory, mounted into the container.
    _config: TempDir,
}

/// Anonymous clients allowed, a plain listener on 1883 and a TLS listener
/// on 8883 with the server certificate of `pki`.
const OPEN_CONFIG: &str = "\
listener 1883
listener 8883
cafile /mosquitto/config/ca.pem
certfile /mosquitto/config/server.pem
keyfile /mosquitto/config/server.key
allow_anonymous true
log_dest stdout
";

/// The dynamic-security plugin holding every client, as the server's
/// broker is set up.
const DYNAMIC_SECURITY_CONFIG: &str = "\
listener 1883
allow_anonymous false
plugin /usr/lib/mosquitto_dynamic_security.so
plugin_opt_config_file /mosquitto/data/dynamic-security.json
log_dest stdout
";

impl Mosquitto {
    /// A broker that lets anyone in, over plain TCP and over TLS.
    pub(crate) fn open(pki: &Pki) -> Self {
        let config = config_directory(OPEN_CONFIG);
        write_readable(config.path(), "ca.pem", &pki.ca_pem);
        write_readable(config.path(), "server.pem", &pki.server_cert_pem);
        write_readable(config.path(), "server.key", &pki.server_key_pem);
        Self::start(
            config,
            &["mosquitto", "-c", "/mosquitto/config/mosquitto.conf"],
        )
    }

    /// A broker whose dynamic-security plugin knows only its administrator.
    pub(crate) fn with_dynamic_security() -> Self {
        let config = config_directory(DYNAMIC_SECURITY_CONFIG);
        let file = "/mosquitto/data/dynamic-security.json";
        let script = format!(
            "mosquitto_ctrl dynsec init {file} {ADMIN} {ADMIN_PASSWORD} >/dev/null \
             && chown mosquitto:mosquitto {file} \
             && exec mosquitto -c /mosquitto/config/mosquitto.conf"
        );
        Self::start(config, &["sh", "-c", &script])
    }

    fn start(config: TempDir, command: &[&str]) -> Self {
        let mount = format!("{}:/mosquitto/config:ro", config.path().display());
        let mut arguments = vec!["run", "-d", "--rm", "-p", "127.0.0.1::1883", "-p"];
        arguments.extend(["127.0.0.1::8883", "-v", &mount, IMAGE]);
        arguments.extend(command);
        let id = docker(&arguments);
        let broker = Self {
            id,
            _config: config,
        };
        broker.wait_until_running();
        broker
    }

    /// The host port the container's `port` is published on.
    pub(crate) fn port(&self, port: u16) -> u16 {
        let mapping = docker(&["port", &self.id, &format!("{port}/tcp")]);
        let first = mapping.lines().next().unwrap();
        first.rsplit(':').next().unwrap().parse().unwrap()
    }

    /// Runs a dynamic-security command as the administrator.
    pub(crate) fn dynamic_security(&self, command: &[&str]) {
        let mut arguments = vec!["exec", &self.id, "mosquitto_ctrl", "-u", ADMIN, "-P"];
        arguments.extend([ADMIN_PASSWORD, "dynsec"]);
        arguments.extend(command);
        docker(&arguments);
    }

    fn wait_until_running(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let logs = Command::new("docker")
                .args(["logs", &self.id])
                .output()
                .unwrap();
            if String::from_utf8_lossy(&logs.stdout).contains(" running") {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("mosquitto did not start: {}", docker(&["logs", &self.id]));
    }
}

impl Drop for Mosquitto {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.id]).output();
    }
}

fn config_directory(config: &str) -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    write_readable(directory.path(), "mosquitto.conf", config);
    directory
}

/// Writes `contents` to `name` readable by the broker's own user.
fn write_readable(directory: &Path, name: &str, contents: &str) {
    let path = directory.join(name);
    fs::write(&path, contents).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
}

/// Runs docker with `arguments` and returns what it printed; panics with
/// its error output when it fails.
fn docker(arguments: &[&str]) -> String {
    let output = Command::new("docker").args(arguments).output().unwrap();
    assert!(
        output.status.success(),
        "docker {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
