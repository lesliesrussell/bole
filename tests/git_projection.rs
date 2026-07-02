// bole-6c6
use bole::object::{EntryKind, Snapshot, TreeEntry};
use bole::refs::{RefName, TimelinePolicy};
use bole::{Accessor, PathAcl, PathRole, Permission, Repository, TimelineRole};
use bytes::Bytes;
use flate2::read::ZlibDecoder;
use std::collections::BTreeMap;
use std::io::Read;
use tempfile::tempdir;

// ---------- helpers ----------

fn read_git_object_text(repo_path: &std::path::Path, sha: &str) -> String {
    let obj_path = repo_path.join("objects").join(&sha[..2]).join(&sha[2..]);
    let file = std::fs::File::open(&obj_path)
        .unwrap_or_else(|_| panic!("object missing: {sha}"));
    let mut decoder = ZlibDecoder::new(file);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).unwrap();
    let nul = raw.iter().position(|&b| b == 0).unwrap();
    String::from_utf8(raw[nul + 1..].to_vec()).unwrap()
}

fn read_tree_bytes(repo_path: &std::path::Path, tree_sha: &str) -> Vec<u8> {
    let obj_path = repo_path
        .join("objects")
        .join(&tree_sha[..2])
        .join(&tree_sha[2..]);
    let file = std::fs::File::open(&obj_path).unwrap();
    let mut decoder = ZlibDecoder::new(file);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).unwrap();
    let nul = raw.iter().position(|&b| b == 0).unwrap();
    raw[nul + 1..].to_vec()
}

fn tree_entry_names(tree_bytes: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut i = 0;
    while i < tree_bytes.len() {
        let sp = match tree_bytes[i..].iter().position(|&b| b == b' ') {
            Some(p) => p,
            None => break,
        };
        let after_mode = i + sp + 1;
        let nl = match tree_bytes[after_mode..].iter().position(|&b| b == 0) {
            Some(p) => p,
            None => break,
        };
        let name = std::str::from_utf8(&tree_bytes[after_mode..after_mode + nl]).unwrap();
        names.push(name.to_owned());
        i = after_mode + nl + 1 + 20;
    }
    names
}

fn commit_parent_sha(commit_text: &str) -> Option<String> {
    commit_text
        .lines()
        .find(|l| l.starts_with("parent "))
        .map(|l| l.strip_prefix("parent ").unwrap().trim().to_owned())
}

fn commit_tree_sha(commit_text: &str) -> String {
    commit_text
        .lines()
        .find(|l| l.starts_with("tree "))
        .unwrap()
        .strip_prefix("tree ")
        .unwrap()
        .trim()
        .to_owned()
}

fn count_chain(repo_path: &std::path::Path, start: &str) -> usize {
    let mut count = 0;
    let mut current = start.trim().to_owned();
    loop {
        let text = read_git_object_text(repo_path, &current);
        count += 1;
        match commit_parent_sha(&text) {
            Some(p) => current = p,
            None => break,
        }
    }
    count
}

fn object_exists(repo_path: &std::path::Path, sha: &str) -> bool {
    repo_path
        .join("objects")
        .join(&sha[..2])
        .join(&sha[2..])
        .exists()
}

// ---------- T7 tests ----------

/// T7-1: A linear 3-commit timeline projects a branch ref with correct parent chain.
#[tokio::test]
async fn t7_linear_timeline_projects() {
    let repo = Repository::memory();

    let b1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
    let mut e1 = BTreeMap::new();
    e1.insert(
        "app.rs".into(),
        TreeEntry { id: b1, kind: EntryKind::Blob },
    );
    let t1 = repo.objects.put_tree(e1).await.unwrap();
    let s1 = repo
        .objects
        .put_snapshot(Snapshot {
            root: t1,
            parents: vec![],
            author: "alice".into(),
            created_at: 1000,
            message: "first".into(),
        })
        .await
        .unwrap();

    let b2 = repo.objects.put_blob(Bytes::from("v2")).await.unwrap();
    let mut e2 = BTreeMap::new();
    e2.insert(
        "app.rs".into(),
        TreeEntry { id: b2, kind: EntryKind::Blob },
    );
    let t2 = repo.objects.put_tree(e2).await.unwrap();
    let s2 = repo
        .objects
        .put_snapshot(Snapshot {
            root: t2,
            parents: vec![s1],
            author: "alice".into(),
            created_at: 2000,
            message: "second".into(),
        })
        .await
        .unwrap();

    let b3 = repo.objects.put_blob(Bytes::from("v3")).await.unwrap();
    let mut e3 = BTreeMap::new();
    e3.insert(
        "app.rs".into(),
        TreeEntry { id: b3, kind: EntryKind::Blob },
    );
    let t3 = repo.objects.put_tree(e3).await.unwrap();
    let s3 = repo
        .objects
        .put_snapshot(Snapshot {
            root: t3,
            parents: vec![s2],
            author: "alice".into(),
            created_at: 3000,
            message: "third".into(),
        })
        .await
        .unwrap();

    let name = RefName::new("main").unwrap();
    repo.refs
        .create_timeline(
            name,
            s3,
            TimelinePolicy::Unrestricted,
            3000,
            "persistent".into(),
            None,
        )
        .unwrap();

    let dir = tempdir().unwrap();
    let target = dir.path().join("out.git");
    bole::project_to_git(&repo, &target, &Accessor::privileged())
        .await
        .unwrap();

    // branch ref exists
    let ref_path = target.join("refs/heads/main");
    assert!(ref_path.exists(), "refs/heads/main missing");

    // 3-commit chain
    let head_sha = std::fs::read_to_string(&ref_path).unwrap();
    let head_sha = head_sha.trim();
    assert_eq!(count_chain(&target, head_sha), 3, "expected 3 commits");

    // head commit author contains bole@local
    let head_text = read_git_object_text(&target, head_sha);
    assert!(
        head_text.contains("<bole@local>"),
        "author missing bole@local: {head_text}"
    );

    // second commit is the parent of third
    let parent_sha = commit_parent_sha(&head_text).expect("head commit has no parent");
    let parent_text = read_git_object_text(&target, &parent_sha);
    assert!(
        parent_text.contains("second"),
        "parent commit message wrong: {parent_text}"
    );
}

/// T7-2: ACL-filtered projection excludes private paths.
#[tokio::test]
async fn t7_private_paths_excluded() {
    let repo = Repository::memory();

    // Mark secrets/** as ACL-protected (only accessible to privileged readers)
    repo.acls
        .set_path_acl(PathAcl { glob: "secrets/**".into() })
        .unwrap();

    let pub_blob = repo.objects.put_blob(Bytes::from("public")).await.unwrap();
    let sec_blob = repo.objects.put_blob(Bytes::from("secret")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert(
        "src/app.rs".into(),
        TreeEntry { id: pub_blob, kind: EntryKind::Blob },
    );
    entries.insert(
        "secrets/key".into(),
        TreeEntry { id: sec_blob, kind: EntryKind::Blob },
    );
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo
        .objects
        .put_snapshot(Snapshot {
            root: tree_id,
            parents: vec![],
            author: "bob".into(),
            created_at: 1,
            message: "init".into(),
        })
        .await
        .unwrap();
    repo.refs
        .create_timeline(
            RefName::new("main").unwrap(),
            snap_id,
            TimelinePolicy::Unrestricted,
            1,
            "persistent".into(),
            None,
        )
        .unwrap();

    // Projection 1: accessor that can only read src/**
    let restricted = Accessor::new()
        .with_timeline_role(TimelineRole {
            pattern: "**".into(),
            permission: Permission::Read,
        })
        .with_path_role(PathRole {
            glob: "src/**".into(),
            permission: Permission::Read,
        });
    let dir1 = tempdir().unwrap();
    let t1 = dir1.path().join("out.git");
    bole::project_to_git(&repo, &t1, &restricted).await.unwrap();

    let sha1 = std::fs::read_to_string(t1.join("refs/heads/main")).unwrap();
    let tree1 = read_tree_bytes(
        &t1,
        &commit_tree_sha(&read_git_object_text(&t1, sha1.trim())),
    );
    let names1 = tree_entry_names(&tree1);
    assert!(
        names1.contains(&"src".to_owned()),
        "src dir missing from restricted projection"
    );
    assert!(
        !names1.contains(&"secrets".to_owned()),
        "secrets dir must be excluded"
    );

    // Projection 2: privileged accessor sees both
    let dir2 = tempdir().unwrap();
    let t2 = dir2.path().join("out.git");
    bole::project_to_git(&repo, &t2, &Accessor::privileged())
        .await
        .unwrap();

    let sha2 = std::fs::read_to_string(t2.join("refs/heads/main")).unwrap();
    let tree2 = read_tree_bytes(
        &t2,
        &commit_tree_sha(&read_git_object_text(&t2, sha2.trim())),
    );
    let names2 = tree_entry_names(&tree2);
    assert!(names2.contains(&"src".to_owned()));
    assert!(
        names2.contains(&"secrets".to_owned()),
        "privileged projection must include secrets"
    );
}

/// T7-3: Two branches sharing an ancestor write the ancestor commit exactly once.
#[tokio::test]
async fn t7_shared_ancestry_deduplicated() {
    let repo = Repository::memory();

    // Common base snapshot
    let b = repo.objects.put_blob(Bytes::from("base")).await.unwrap();
    let mut be = BTreeMap::new();
    be.insert(
        "base.rs".into(),
        TreeEntry { id: b, kind: EntryKind::Blob },
    );
    let bt = repo.objects.put_tree(be).await.unwrap();
    let base = repo
        .objects
        .put_snapshot(Snapshot {
            root: bt,
            parents: vec![],
            author: "".into(),
            created_at: 1,
            message: "base".into(),
        })
        .await
        .unwrap();

    // Branch A: one commit on top of base
    let ba = repo.objects.put_blob(Bytes::from("a")).await.unwrap();
    let mut ae = BTreeMap::new();
    ae.insert(
        "a.rs".into(),
        TreeEntry { id: ba, kind: EntryKind::Blob },
    );
    let at = repo.objects.put_tree(ae).await.unwrap();
    let sa = repo
        .objects
        .put_snapshot(Snapshot {
            root: at,
            parents: vec![base],
            author: "".into(),
            created_at: 2,
            message: "a".into(),
        })
        .await
        .unwrap();

    // Branch B: one commit on top of base
    let bb = repo.objects.put_blob(Bytes::from("b")).await.unwrap();
    let mut be2 = BTreeMap::new();
    be2.insert(
        "b.rs".into(),
        TreeEntry { id: bb, kind: EntryKind::Blob },
    );
    let bt2 = repo.objects.put_tree(be2).await.unwrap();
    let sb = repo
        .objects
        .put_snapshot(Snapshot {
            root: bt2,
            parents: vec![base],
            author: "".into(),
            created_at: 3,
            message: "b".into(),
        })
        .await
        .unwrap();

    repo.refs
        .create_timeline(
            RefName::new("branch-a").unwrap(),
            sa,
            TimelinePolicy::Unrestricted,
            2,
            "persistent".into(),
            None,
        )
        .unwrap();
    repo.refs
        .create_timeline(
            RefName::new("branch-b").unwrap(),
            sb,
            TimelinePolicy::Unrestricted,
            3,
            "persistent".into(),
            None,
        )
        .unwrap();

    let dir = tempdir().unwrap();
    let target = dir.path().join("out.git");
    bole::project_to_git(&repo, &target, &Accessor::privileged())
        .await
        .unwrap();

    assert!(target.join("refs/heads/branch-a").exists());
    assert!(target.join("refs/heads/branch-b").exists());

    let sha_a = std::fs::read_to_string(target.join("refs/heads/branch-a")).unwrap();
    let sha_b = std::fs::read_to_string(target.join("refs/heads/branch-b")).unwrap();

    // Each branch head should have the same parent commit (the common ancestor)
    let text_a = read_git_object_text(&target, sha_a.trim());
    let text_b = read_git_object_text(&target, sha_b.trim());
    let parent_a = commit_parent_sha(&text_a).expect("branch-a has no parent");
    let parent_b = commit_parent_sha(&text_b).expect("branch-b has no parent");
    assert_eq!(
        parent_a, parent_b,
        "both branches should share the same ancestor commit SHA"
    );

    // The ancestor object exists (file-based presence check)
    assert!(
        object_exists(&target, &parent_a),
        "common ancestor object file missing"
    );
}

/// T7-4: A bole tag projects to refs/tags/{name} pointing at the correct commit.
#[tokio::test]
async fn t7_tags_projected() {
    let repo = Repository::memory();

    let b1 = repo.objects.put_blob(Bytes::from("v1")).await.unwrap();
    let mut e1 = BTreeMap::new();
    e1.insert(
        "main.rs".into(),
        TreeEntry { id: b1, kind: EntryKind::Blob },
    );
    let t1 = repo.objects.put_tree(e1).await.unwrap();
    let s1 = repo
        .objects
        .put_snapshot(Snapshot {
            root: t1,
            parents: vec![],
            author: "".into(),
            created_at: 1,
            message: "initial".into(),
        })
        .await
        .unwrap();

    let b2 = repo.objects.put_blob(Bytes::from("v2")).await.unwrap();
    let mut e2 = BTreeMap::new();
    e2.insert(
        "main.rs".into(),
        TreeEntry { id: b2, kind: EntryKind::Blob },
    );
    let t2 = repo.objects.put_tree(e2).await.unwrap();
    let s2 = repo
        .objects
        .put_snapshot(Snapshot {
            root: t2,
            parents: vec![s1],
            author: "".into(),
            created_at: 2,
            message: "second".into(),
        })
        .await
        .unwrap();

    repo.refs
        .create_timeline(
            RefName::new("main").unwrap(),
            s2,
            TimelinePolicy::Unrestricted,
            2,
            "persistent".into(),
            None,
        )
        .unwrap();
    // Tag pointing at the first snapshot
    repo.refs
        .create_tag(RefName::new("v1.0").unwrap(), s1, None, 1)
        .unwrap();

    let dir = tempdir().unwrap();
    let target = dir.path().join("out.git");
    bole::project_to_git(&repo, &target, &Accessor::privileged())
        .await
        .unwrap();

    let tag_path = target.join("refs/tags/v1.0");
    assert!(tag_path.exists(), "refs/tags/v1.0 missing");

    // tag SHA should equal the git commit for s1 (the first snapshot)
    let tag_sha = std::fs::read_to_string(&tag_path).unwrap();
    let tag_sha = tag_sha.trim();
    let tag_text = read_git_object_text(&target, tag_sha);
    assert!(
        tag_text.contains("initial"),
        "tag should point to initial commit: {tag_text}"
    );

    // second commit exists and the branch points to it
    let head_sha = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
    let head_sha = head_sha.trim();
    let head_text = read_git_object_text(&target, head_sha);
    assert!(
        head_text.contains("second"),
        "head should be second commit: {head_text}"
    );

    // the two SHAs should differ
    assert_ne!(tag_sha, head_sha, "tag and head should point to different commits");
}

/// T7-5: Secret objects stored in a tree are silently excluded; project_to_git returns Ok.
#[tokio::test]
async fn t7_secret_entries_skipped() {
    let repo = Repository::memory();

    // Store a secret; its ObjectId is used as a tree entry with Blob kind
    let key = [0u8; 32];
    let secret_id = repo.objects.put_secret(b"very-secret", &key).await.unwrap();

    let safe_blob = repo.objects.put_blob(Bytes::from("safe content")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert(
        "safe.rs".into(),
        TreeEntry { id: safe_blob, kind: EntryKind::Blob },
    );
    entries.insert(
        "secret.key".into(),
        TreeEntry { id: secret_id, kind: EntryKind::Blob },
    );
    let tree_id = repo.objects.put_tree(entries).await.unwrap();
    let snap_id = repo
        .objects
        .put_snapshot(Snapshot {
            root: tree_id,
            parents: vec![],
            author: "".into(),
            created_at: 1,
            message: "with secret".into(),
        })
        .await
        .unwrap();
    repo.refs
        .create_timeline(
            RefName::new("main").unwrap(),
            snap_id,
            TimelinePolicy::Unrestricted,
            1,
            "persistent".into(),
            None,
        )
        .unwrap();

    let dir = tempdir().unwrap();
    let target = dir.path().join("out.git");
    let result = bole::project_to_git(&repo, &target, &Accessor::privileged()).await;
    assert!(
        result.is_ok(),
        "project_to_git must not error on secret entries: {:?}",
        result.err()
    );

    let head_sha = std::fs::read_to_string(target.join("refs/heads/main")).unwrap();
    let tree_sha = commit_tree_sha(&read_git_object_text(&target, head_sha.trim()));
    let tree_bytes = read_tree_bytes(&target, &tree_sha);
    let names = tree_entry_names(&tree_bytes);

    assert!(names.contains(&"safe.rs".to_owned()), "safe.rs must be present");
    assert!(
        !names.contains(&"secret.key".to_owned()),
        "secret.key must be excluded"
    );
}

// bole-x8w
/// An accessor with only a PATH grant (no timeline grant) must still see an
/// unprotected (public) timeline in the export — public visibility mirrors how
/// public files are shown. Before bole-x8w, project_to_git filtered timelines via
/// accessor.can_read_timeline (accessor's own rules, no public/bottom
/// short-circuit) and produced an empty repo for such an accessor.
#[tokio::test]
async fn export_includes_public_timeline_for_path_only_accessor() {
    let repo = Repository::memory();
    let blob = repo.objects.put_blob(Bytes::from("hello")).await.unwrap();
    let mut entries = BTreeMap::new();
    entries.insert("src/main.rs".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
    let tree = repo.objects.put_tree(entries).await.unwrap();
    let snap = repo
        .objects
        .put_snapshot(Snapshot {
            root: tree,
            parents: vec![],
            author: "a".into(),
            created_at: 1,
            message: "init".into(),
        })
        .await
        .unwrap();
    repo.refs
        .create_timeline(
            RefName::new("main").unwrap(),
            snap,
            TimelinePolicy::Unrestricted,
            1,
            "persistent".into(),
            None,
        )
        .unwrap();

    // Path-only accessor: read everything by path, but NO timeline grant.
    let accessor =
        Accessor::new().with_path_role(PathRole { glob: "**".into(), permission: Permission::Read });
    let dir = tempdir().unwrap();
    let target = dir.path().join("out.git");
    bole::project_to_git(&repo, &target, &accessor).await.unwrap();

    assert!(
        target.join("refs/heads/main").exists(),
        "public timeline must be exported even without an explicit timeline grant"
    );
}
