// bole-6i1
//! End-to-end integration tests for the collab CLI authoring workflow:
//! `bole profile set/show` and `bole trust follow/list`.

use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bole"))
}

fn run(dir: &Path, args: &[&str], seed: Option<&str>) -> std::process::Output {
    let mut c = bin();
    c.args(args).current_dir(dir);
    if let Some(s) = seed {
        c.env("BOLE_COLLAB_KEY", s);
    }
    c.output().unwrap()
}

fn ok(dir: &Path, args: &[&str], seed: Option<&str>) -> std::process::Output {
    let out = run(dir, args, seed);
    assert!(out.status.success(), "cmd {args:?} failed: {out:?}");
    out
}

#[test]
fn cli_profile_set_and_show() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "aa".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Alice", "--bio", "hi"], Some(&seed));
    let show = ok(w, &["profile", "show", "--json"], Some(&seed));
    let v: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(v["display_name"], "Alice");
    // Re-setting bumps seq monotonically.
    ok(w, &["profile", "set", "--display-name", "Alice2"], Some(&seed));
    let show2 = ok(w, &["profile", "show", "--json"], Some(&seed));
    let v2: serde_json::Value = serde_json::from_slice(&show2.stdout).unwrap();
    assert_eq!(v2["display_name"], "Alice2");
    assert!(v2["seq"].as_u64().unwrap() > v["seq"].as_u64().unwrap());
}

#[test]
fn cli_trust_follow_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "bb".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Me"], Some(&seed));
    let peer = "cc".repeat(32); // a 64-hex key
    ok(w, &["trust", "follow", &peer], Some(&seed));
    let list = ok(w, &["trust", "list", "--json"], Some(&seed));
    assert!(String::from_utf8_lossy(&list.stdout).contains(&peer[..8]));
}

// bole-6i1
#[test]
fn cli_trust_vouch_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "ab".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Me"], Some(&seed));
    let peer = "cd".repeat(32);
    ok(w, &["trust", "vouch", &peer, "--name", "buddy"], Some(&seed));
    let list = ok(w, &["trust", "list", "--json"], Some(&seed));
    let s = String::from_utf8_lossy(&list.stdout);
    assert!(s.contains(&peer[..8]), "vouched key present: {s}");
    assert!(s.contains("buddy"), "petname present: {s}");
}

// bole-cyw
#[test]
fn cli_query_shows_petname_and_reach() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path();
    ok(w, &["init", "."], None);
    let seed = "a1".repeat(32);
    ok(w, &["profile", "set", "--display-name", "Myself"], Some(&seed));
    let out = ok(w, &["discover", "query", "Myself", "--json"], Some(&seed));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let row = &v[0];
    assert_eq!(row["reach"], "self");
    assert!(row.get("display_name").is_some(), "display_name field present");
    assert!(row.get("petname").is_some(), "petname field present (may be null)");
    assert!(row.get("trust_path").is_some(), "trust_path field present");
}

// bole-1n7
#[test]
fn cli_discover_pull_query_e2e() {
    use std::process::Stdio;
    // Server repo: publish a profile, serve on a fixed loopback port.
    let stmp = tempfile::tempdir().unwrap();
    let s = stmp.path();
    ok(s, &["init", "."], None);
    let sseed = "dd".repeat(32);
    ok(s, &["profile", "set", "--display-name", "Server"], Some(&sseed));

    let addr = "127.0.0.1:47653";
    let mut server = bin();
    server.args(["node", "serve", "--listen", addr]).current_dir(s)
        .stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = server.spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(400)); // let it bind

    // Client repo: pull the server, follow the pulled key, then query. A peer
    // is only discoverable once it is inside the client's follow-neighborhood,
    // so the pulled key is followed before querying.
    let ctmp = tempfile::tempdir().unwrap();
    let c = ctmp.path();
    ok(c, &["init", "."], None);
    let cseed = "ee".repeat(32);
    ok(c, &["profile", "set", "--display-name", "Client"], Some(&cseed));
    let pull = ok(c, &["discover", "pull", addr, "--json"], Some(&cseed));
    let pv: serde_json::Value = serde_json::from_slice(&pull.stdout).unwrap();
    let peer_key = pv["pulled"].as_str().unwrap().to_string();
    ok(c, &["trust", "follow", &peer_key], Some(&cseed));
    let q = ok(c, &["discover", "query", "Server", "--json"], Some(&cseed));
    let _ = child.kill();
    let _ = child.wait(); // reap the daemon so no zombie is left behind
    assert!(String::from_utf8_lossy(&q.stdout).contains("Server"), "peer discoverable: {}", String::from_utf8_lossy(&q.stdout));
}
