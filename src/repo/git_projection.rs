// bole-6bd
// bole-4hy
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use gix::prelude::Write as _;

use crate::acl::Accessor;
use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};
use crate::refs::Ref;
use crate::repo::Repository;

// bole-p8u
/// Projects all timelines visible to `accessor` from `repo` into a bare Git
/// repository at `target_path`, creating it if it does not already exist.
///
/// Blobs, trees, and snapshots are translated to native Git objects; each
/// timeline head becomes a Git branch ref.  Only paths the accessor can read
/// are included in the projected commits.
pub async fn project_to_git(
    repo: &Repository,
    target_path: &Path,
    accessor: &Accessor,
) -> Result<()> {
    // bole-68s
    // Pass 1: open existing bare repo or create a new one (idempotent)
    let git_repo = if target_path.exists() {
        gix::open(target_path)
            .map_err(|e| Error::GitProjection(format!("path exists but is not a git repo: {e}")))?
    } else {
        gix::init_bare(target_path)
            .map_err(|e| Error::GitProjection(e.to_string()))?
    };

    // Pass 2: collect timeline heads, then topo-sort reachable snapshots
    let all_refs = repo.refs.list("")?;
    let mut timeline_heads: Vec<(crate::refs::RefName, ObjectId)> = Vec::new();
    for name in &all_refs {
        if let Some(Ref::Timeline(tl)) = repo.refs.get(name)? {
            if accessor.can_read_timeline(name.as_str()) {
                timeline_heads.push((name.clone(), tl.head));
            }
        }
    }
    let starts: Vec<ObjectId> = timeline_heads.iter().map(|(_, h)| *h).collect();
    let ordered = collect_topo(&repo.objects, starts).await?;

    // Pass 3: write blobs, trees, commits; build bole → git id map
    // bole-fo2
    let lattice = repo.acls.lattice()?;
    let rules = repo.acls.label_ruleset()?;
    let mut id_map: HashMap<ObjectId, gix::ObjectId> = HashMap::new();
    for snap_id in &ordered {
        let snap = match repo.objects.get(snap_id).await? {
            Some(Object::Snapshot(s)) => s,
            _ => continue,
        };
        let mut flat = BTreeMap::new();
        // bole-fo2
        super::walk_tree_filtered(
            &repo.objects, &lattice, &rules, snap.root, "", accessor, &mut flat,
        ).await?;
        let git_tree = write_git_tree_level(&flat, &repo.objects, &git_repo).await?;

        let parents: Vec<gix::ObjectId> =
            snap.parents.iter().filter_map(|p| id_map.get(p)).cloned().collect();
        let author_name = if snap.author.is_empty() { "bole" } else { snap.author.as_str() };
        let identity = format!("{} <bole@local> {} +0000", author_name, snap.created_at);
        let commit_bytes = encode_commit(git_tree, &parents, &identity, &snap.message);
        let git_commit = git_repo
            .objects
            .write_buf(gix::object::Kind::Commit, &commit_bytes)
            .map_err(|e| Error::GitProjection(e.to_string()))?;
        id_map.insert(*snap_id, git_commit);
    }

    // Pass 4: write branch refs
    for (name, head) in &timeline_heads {
        if let Some(git_id) = id_map.get(head) {
            write_loose_ref(target_path, &format!("refs/heads/{}", name.as_str()), git_id)?;
        }
    }

    // Pass 5: write tag refs (lightweight; target must be in id_map)
    for name in &all_refs {
        if let Some(Ref::Tag(tag)) = repo.refs.get(name)? {
            if let Some(git_id) = id_map.get(&tag.target) {
                write_loose_ref(target_path, &format!("refs/tags/{}", name.as_str()), git_id)?;
            }
        }
    }

    // bole-fv3
    // Pass 6: set HEAD to the first projected timeline branch
    if let Some((branch_name, _)) = timeline_heads.iter().find(|(_, h)| id_map.contains_key(h)) {
        let head_content = format!("ref: refs/heads/{}\n", branch_name.as_str());
        std::fs::write(target_path.join("HEAD"), head_content.as_bytes())?;
    }

    Ok(())
}

/// DFS post-order topological sort of all snapshots reachable from `starts`.
/// Parents always precede their children in the result.
async fn collect_topo(
    objects: &crate::store::ObjectStore,
    starts: Vec<ObjectId>,
) -> Result<Vec<ObjectId>> {
    let mut visited: HashSet<ObjectId> = HashSet::new();
    let mut result: Vec<ObjectId> = Vec::new();
    // stack entries: (id, finishing)
    // When finishing=true, append to result. When false, push with finishing=true
    // then push unvisited parents (so parents are processed before the node).
    let mut stack: Vec<(ObjectId, bool)> =
        starts.iter().map(|id| (*id, false)).collect();
    while let Some((id, finishing)) = stack.pop() {
        if finishing {
            result.push(id);
            continue;
        }
        if !visited.insert(id) {
            continue; // already processed via another path
        }
        stack.push((id, true));
        if let Some(Object::Snapshot(snap)) = objects.get(&id).await? {
            for parent in snap.parents.iter().rev() {
                stack.push((*parent, false));
            }
        }
    }
    Ok(result)
}

/// Convert a flat ACL-filtered path map (from walk_tree_filtered) to nested
/// git tree objects. Returns the root git tree's ObjectId.
async fn write_git_tree_level(
    flat: &BTreeMap<String, ObjectId>,
    objects: &crate::store::ObjectStore,
    git_repo: &gix::Repository,
) -> Result<gix::ObjectId> {
    let mut entries: Vec<(String, bool, gix::ObjectId)> = Vec::new();
    let mut dirs_seen: HashSet<String> = HashSet::new();

    for (path, blob_id) in flat {
        if let Some(slash) = path.find('/') {
            let dir = &path[..slash];
            if dirs_seen.insert(dir.to_owned()) {
                let prefix = format!("{}/", dir);
                let sub_flat: BTreeMap<String, ObjectId> = flat
                    .iter()
                    .filter(|(k, _)| k.starts_with(&prefix))
                    .map(|(k, v)| (k[prefix.len()..].to_owned(), *v))
                    .collect();
                let sub_id = Box::pin(write_git_tree_level(&sub_flat, objects, git_repo)).await?;
                entries.push((dir.to_owned(), true, sub_id));
            }
        } else {
            // Secret, EnvOverlay, or missing — skip silently; only write real blobs
            if let Some(Object::Blob(b)) = objects.get(blob_id).await? {
                let git_blob = git_repo
                    .objects
                    .write_buf(gix::object::Kind::Blob, b.data.as_ref())
                    .map_err(|e| Error::GitProjection(e.to_string()))?;
                entries.push((path.clone(), false, git_blob));
            }
        }
    }

    let tree_bytes = encode_tree(&entries);
    git_repo
        .objects
        .write_buf(gix::object::Kind::Tree, &tree_bytes)
        .map_err(|e| Error::GitProjection(e.to_string()))
}

/// Encode git tree binary format.
/// Entry format: `"{mode} {name}\0{20-byte-sha}"`
/// Sort order: directories sort as if they have a trailing `/`.
fn encode_tree(entries: &[(String, bool, gix::ObjectId)]) -> Vec<u8> {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|(a_name, a_dir, _), (b_name, b_dir, _)| {
        let a_key = if *a_dir { format!("{}/", a_name) } else { a_name.clone() };
        let b_key = if *b_dir { format!("{}/", b_name) } else { b_name.clone() };
        a_key.as_bytes().cmp(b_key.as_bytes())
    });
    let mut buf = Vec::new();
    for (name, is_dir, sha) in &sorted {
        let mode: &[u8] = if *is_dir { b"40000" } else { b"100644" };
        buf.extend_from_slice(mode);
        buf.push(b' ');
        buf.extend_from_slice(name.as_bytes());
        buf.push(0u8);
        buf.extend_from_slice(sha.as_bytes());
    }
    buf
}

/// Encode git commit text format (no leading header — gix adds it via write_buf).
fn encode_commit(
    tree: gix::ObjectId,
    parents: &[gix::ObjectId],
    identity: &str,
    message: &str,
) -> Vec<u8> {
    let mut s = format!("tree {}\n", tree);
    for p in parents {
        s.push_str(&format!("parent {}\n", p));
    }
    s.push_str(&format!("author {}\n", identity));
    s.push_str(&format!("committer {}\n", identity));
    s.push('\n');
    s.push_str(message);
    if !message.ends_with('\n') {
        s.push('\n');
    }
    s.into_bytes()
}

/// Write a loose ref file in a bare git repo.
/// `ref_name` examples: `"refs/heads/main"`, `"refs/tags/v1.0"`.
/// Content: `"{sha_hex}\n"`.
fn write_loose_ref(repo_path: &Path, ref_name: &str, sha: &gix::ObjectId) -> Result<()> {
    let ref_path = repo_path.join(ref_name);
    if let Some(parent) = ref_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&ref_path, format!("{}\n", sha)).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::Accessor;
    use crate::object::{EntryKind, Snapshot, TreeEntry};
    use crate::refs::{RefName, TimelinePolicy};
    use bytes::Bytes;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    // ---------- repo builders ----------

    async fn linear_repo() -> (Repository, ObjectId, ObjectId, ObjectId) {
        let repo = Repository::memory();

        let b1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
        let mut e1 = BTreeMap::new();
        e1.insert("app.rs".into(), TreeEntry { id: b1, kind: EntryKind::Blob });
        let t1 = repo.objects.put_tree(e1).await.unwrap();
        let s1 = repo.objects.put_snapshot(Snapshot {
            root: t1, parents: vec![],
            author: "alice".into(), created_at: 1000, message: "first".into(),
        }).await.unwrap();

        let b2 = repo.objects.put_blob(Bytes::from("v2")).await.unwrap();
        let mut e2 = BTreeMap::new();
        e2.insert("app.rs".into(), TreeEntry { id: b2, kind: EntryKind::Blob });
        let t2 = repo.objects.put_tree(e2).await.unwrap();
        let s2 = repo.objects.put_snapshot(Snapshot {
            root: t2, parents: vec![s1],
            author: "alice".into(), created_at: 2000, message: "second".into(),
        }).await.unwrap();

        let b3 = repo.objects.put_blob(Bytes::from("v3")).await.unwrap();
        let mut e3 = BTreeMap::new();
        e3.insert("app.rs".into(), TreeEntry { id: b3, kind: EntryKind::Blob });
        let t3 = repo.objects.put_tree(e3).await.unwrap();
        let s3 = repo.objects.put_snapshot(Snapshot {
            root: t3, parents: vec![s2],
            author: "alice".into(), created_at: 3000, message: "third".into(),
        }).await.unwrap();

        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name, s3, TimelinePolicy::Unrestricted, 3000, "persistent".into(), None).unwrap();
        (repo, s1, s2, s3)
    }

    // ---------- test helpers ----------

    /// Walk the parent chain in the exported git repo, counting commits.
    fn count_commit_chain(repo_path: &std::path::Path, start_sha: &str) -> usize {
        let mut count = 0usize;
        let mut current = start_sha.trim().to_owned();
        loop {
            let text = read_git_object_text(repo_path, &current);
            count += 1;
            if let Some(line) = text.lines().find(|l| l.starts_with("parent ")) {
                current = line.strip_prefix("parent ").unwrap().trim().to_owned();
            } else {
                break;
            }
        }
        count
    }

    /// Read and zlib-decompress a loose git object, returning its text payload (header stripped).
    fn read_git_object_text(repo_path: &std::path::Path, sha: &str) -> String {
        use flate2::read::ZlibDecoder;
        use std::io::Read;
        let obj_path = repo_path.join("objects").join(&sha[..2]).join(&sha[2..]);
        let file = std::fs::File::open(&obj_path)
            .unwrap_or_else(|_| panic!("object file missing: {}", obj_path.display()));
        let mut decoder = ZlibDecoder::new(file);
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw).unwrap();
        let nul = raw.iter().position(|&b| b == 0).expect("no nul in object");
        String::from_utf8(raw[nul + 1..].to_vec()).unwrap()
    }

    /// Return raw git tree payload bytes for the tree referenced in a commit text.
    fn read_tree_of_commit(repo_path: &std::path::Path, commit_sha: &str) -> Vec<u8> {
        use flate2::read::ZlibDecoder;
        use std::io::Read;
        let commit_text = read_git_object_text(repo_path, commit_sha);
        let tree_line = commit_text.lines().find(|l| l.starts_with("tree ")).unwrap();
        let tree_sha = tree_line.strip_prefix("tree ").unwrap().trim();
        let obj_path = repo_path.join("objects").join(&tree_sha[..2]).join(&tree_sha[2..]);
        let file = std::fs::File::open(&obj_path).unwrap();
        let mut decoder = ZlibDecoder::new(file);
        let mut raw = Vec::new();
        decoder.read_to_end(&mut raw).unwrap();
        let nul = raw.iter().position(|&b| b == 0).unwrap();
        raw[nul + 1..].to_vec()
    }

    /// Return top-level entry names from a raw git tree binary payload.
    fn tree_top_entry_names(tree_data: &[u8]) -> Vec<String> {
        let mut names = Vec::new();
        let mut i = 0;
        while i < tree_data.len() {
            let sp = match tree_data[i..].iter().position(|&b| b == b' ') {
                Some(p) => p,
                None => break,
            };
            let after_mode = i + sp + 1;
            let nl = match tree_data[after_mode..].iter().position(|&b| b == 0) {
                Some(p) => p,
                None => break,
            };
            let name = std::str::from_utf8(&tree_data[after_mode..after_mode + nl]).unwrap();
            names.push(name.to_owned());
            i = after_mode + nl + 1 + 20;
        }
        names
    }

    // ---------- unit tests for helpers ----------

    #[test]
    fn encode_tree_sorts_dirs_with_trailing_slash() {
        let null_id = gix::ObjectId::null(gix::hash::Kind::Sha1);
        let entries = vec![
            ("b.rs".to_owned(), false, null_id),
            ("a".to_owned(), true, null_id),
        ];
        let encoded = encode_tree(&entries);
        // First entry should start with "40000 a\0"
        let first_mode_end = encoded.iter().position(|&b| b == b' ').unwrap();
        assert_eq!(&encoded[..first_mode_end], b"40000");
    }

    #[test]
    fn encode_commit_includes_author_and_parent() {
        let null_id = gix::ObjectId::null(gix::hash::Kind::Sha1);
        let bytes = encode_commit(null_id, &[null_id], "alice <bole@local> 1000 +0000", "msg");
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("author alice <bole@local> 1000 +0000"));
        assert!(text.contains("parent "));
        assert!(text.ends_with("msg\n"));
    }

    #[tokio::test]
    async fn collect_topo_orders_parents_before_children() {
        let repo = Repository::memory();
        let root_id = ObjectId::new([0u8; 32]);
        let s1 = repo.objects.put_snapshot(Snapshot {
            root: root_id, parents: vec![],
            author: "".into(), created_at: 1, message: "".into(),
        }).await.unwrap();
        let s2 = repo.objects.put_snapshot(Snapshot {
            root: root_id, parents: vec![s1],
            author: "".into(), created_at: 2, message: "".into(),
        }).await.unwrap();
        let ordered = collect_topo(&repo.objects, vec![s2]).await.unwrap();
        assert_eq!(ordered, vec![s1, s2]);
    }

    // ---------- integration tests ----------

    #[tokio::test]
    async fn linear_timeline_exports_branch_ref() {
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();
        assert!(target.join("refs").join("heads").join("main").exists());
    }

    #[tokio::test]
    async fn exported_commit_chain_has_three_commits() {
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();
        let sha_hex = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
        let count = count_commit_chain(&target, sha_hex.trim());
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn commit_author_contains_bole_local() {
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();
        let sha_hex = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
        let text = read_git_object_text(&target, sha_hex.trim());
        assert!(text.contains("<bole@local>"), "commit author missing bole@local: {text}");
    }

    #[tokio::test]
    async fn acl_denied_timeline_has_no_branch_ref() {
        use crate::acl::{PathRole, Permission, TimelineRole};
        let dir = tempdir().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        // accessor that cannot read the "main" timeline
        let restricted = Accessor::new()
            .with_timeline_role(TimelineRole { pattern: "other/**".into(), permission: Permission::Read })
            .with_path_role(PathRole { glob: "**".into(), permission: Permission::Read });
        project_to_git(&repo, &target, &restricted).await.unwrap();
        assert!(!target.join("refs/heads/main").exists());
    }

    #[tokio::test]
    async fn secret_entry_silently_skipped_in_tree() {
        let dir = tempdir().unwrap();
        let repo = Repository::memory();
        // Store a secret; put its ObjectId directly into a tree as if it were a blob
        let key = [0u8; 32];
        let secret_id = repo.objects.put_secret(b"top-secret", &key).await.unwrap();
        // Build a tree with one real blob and one secret "blob" (wrong kind)
        let real_blob = repo.objects.put_blob(Bytes::from("safe")).await.unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("safe.rs".into(), TreeEntry { id: real_blob, kind: EntryKind::Blob });
        entries.insert("secret.key".into(), TreeEntry { id: secret_id, kind: EntryKind::Blob });
        let tree_id = repo.objects.put_tree(entries).await.unwrap();
        let snap_id = repo.objects.put_snapshot(Snapshot {
            root: tree_id, parents: vec![],
            author: "".into(), created_at: 1, message: "".into(),
        }).await.unwrap();
        let name = RefName::new("main").unwrap();
        repo.refs.create_timeline(name, snap_id, TimelinePolicy::Unrestricted, 1, "persistent".into(), None).unwrap();

        let target = dir.path().join("out.git");
        let result = project_to_git(&repo, &target, &Accessor::privileged()).await;
        assert!(result.is_ok(), "project_to_git returned error: {:?}", result.err());

        let sha_hex = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
        let tree_bytes = read_tree_of_commit(&target, sha_hex.trim());
        let names = tree_top_entry_names(&tree_bytes);
        assert!(names.contains(&"safe.rs".to_owned()));
        assert!(!names.contains(&"secret.key".to_owned()), "secret entry must be excluded");
    }

    // bole-fv3
    #[tokio::test]
    async fn project_to_git_head_points_to_branch() {
        let dir = tempfile::TempDir::new().unwrap();
        let (repo, _, _, _) = linear_repo().await;
        let target = dir.path().join("out.git");
        project_to_git(&repo, &target, &Accessor::privileged()).await.unwrap();
        let head = std::fs::read_to_string(target.join("HEAD")).unwrap();
        assert!(head.starts_with("ref: refs/heads/"), "HEAD must be a symbolic ref, got: {head}");
        let branch = head.trim().strip_prefix("ref: refs/heads/").unwrap();
        assert!(
            target.join("refs/heads").join(branch).exists(),
            "HEAD points to {branch} but refs/heads/{branch} does not exist"
        );
    }

    // bole-68s
    #[tokio::test]
    async fn project_to_git_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repository::memory();
        let accessor = Accessor::privileged();
        let target = dir.path().join("out.git");
        // First projection creates the repo
        project_to_git(&repo, &target, &accessor).await.unwrap();
        // Second projection on same path must succeed (idempotent open)
        project_to_git(&repo, &target, &accessor).await.unwrap();
    }

    #[tokio::test]
    async fn project_to_git_non_repo_path_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repository::memory();
        let accessor = Accessor::privileged();
        // target_path exists but is a plain directory, not a git repo
        let plain = dir.path().join("not-a-repo");
        std::fs::create_dir(&plain).unwrap();
        std::fs::write(plain.join("garbage.txt"), b"not git").unwrap();
        let result = project_to_git(&repo, &plain, &accessor).await;
        assert!(result.is_err(), "non-repo path must return an error");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not a git repo"), "error should describe the problem, got: {msg}");
    }
}
