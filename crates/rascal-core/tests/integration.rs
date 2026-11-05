use std::path::Path;

use git2::{IndexAddOption, Repository, RepositoryInitOptions, Signature, Time};
use rascal_core::{DiffLayer, FileStatus, GitRepo};
use tempfile::TempDir;

struct TestRepo {
    dir: TempDir,
    repo: Repository,
    clock: i64,
}

impl TestRepo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut options = RepositoryInitOptions::new();
        options.initial_head("main");
        let repo = Repository::init_opts(dir.path(), &options).unwrap();
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test User").unwrap();
            config.set_str("user.email", "test@example.com").unwrap();
        }
        Self {
            dir,
            repo,
            clock: 1_700_000_000,
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.dir.path().join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn write_bytes(&self, path: &str, contents: &[u8]) {
        let path = self.dir.path().join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn commit(&mut self, message: &str) -> rascal_core::Oid {
        let mut index = self.repo.index().unwrap();
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = self.repo.find_tree(tree_id).unwrap();

        let signature =
            Signature::new("Test User", "test@example.com", &Time::new(self.clock, 0)).unwrap();
        self.clock += 60;

        let parent = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok());
        let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
        self.repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .unwrap()
    }

    fn open(&self) -> GitRepo {
        GitRepo::open(self.dir.path()).unwrap()
    }
}

#[test]
fn reads_repository_info_and_history() {
    let mut fixture = TestRepo::new();
    fixture.write("a.txt", "one\n");
    let first = fixture.commit("initial");
    fixture.write("a.txt", "one\ntwo\n");
    let second = fixture.commit("second");

    let repo = fixture.open();
    let info = repo.info().unwrap();
    assert_eq!(
        info.name,
        fixture.path().file_name().unwrap().to_string_lossy()
    );
    assert_eq!(info.head.oid(), Some(second));
    assert!(!info.is_bare);

    let page = repo.history("HEAD", 0, 10).unwrap();
    assert_eq!(page.commits.len(), 2);
    assert_eq!(page.commits[0].oid, second);
    assert_eq!(page.commits[1].oid, first);
    assert!(
        page.commits[0]
            .refs
            .iter()
            .any(|label| matches!(label.kind, rascal_core::RefKind::Head))
    );

    let branches = repo.branches().unwrap();
    assert_eq!(branches.len(), 1);
    assert!(branches[0].is_head);
    assert_eq!(branches[0].tip, second);
}

#[test]
fn reads_commit_diff_and_file_stats() {
    let mut fixture = TestRepo::new();
    fixture.write("a.txt", "one\n");
    fixture.commit("initial");
    fixture.write("a.txt", "one\ntwo\n");
    let second = fixture.commit("second");

    let repo = fixture.open();
    let detail = repo.commit(second).unwrap();
    assert_eq!(detail.files.len(), 1);
    assert_eq!(detail.files[0].path, "a.txt");
    assert_eq!(detail.files[0].status, FileStatus::Modified);
    assert_eq!(detail.files[0].additions, 1);
    assert_eq!(detail.files[0].deletions, 0);

    let diff = repo.commit_diff(second, 0, None, Some("a.txt")).unwrap();
    assert!(!diff.is_binary);
    assert!(
        diff.rows
            .iter()
            .any(|row| row.kind == rascal_core::DiffRowKind::Added && row.text == "two")
    );
    assert!(
        diff.rows
            .iter()
            .any(|row| row.kind == rascal_core::DiffRowKind::HunkHeader)
    );
}

#[test]
fn detects_renames_and_root_diffs() {
    let mut fixture = TestRepo::new();
    fixture.write("old.txt", "hello\n");
    let root = fixture.commit("root");
    std::fs::rename(
        fixture.path().join("old.txt"),
        fixture.path().join("new.txt"),
    )
    .unwrap();
    let renamed = fixture.commit("rename");

    let repo = fixture.open();
    let root_detail = repo.commit(root).unwrap();
    assert!(matches!(root_detail.base, rascal_core::DiffBase::Root));
    assert_eq!(root_detail.files[0].status, FileStatus::Added);

    let detail = repo.commit(renamed).unwrap();
    assert_eq!(detail.files[0].status, FileStatus::Renamed);
    assert_eq!(detail.files[0].path, "new.txt");
    assert_eq!(detail.files[0].old_path.as_deref(), Some("old.txt"));
    let diff = repo
        .commit_diff(renamed, 0, Some("old.txt"), Some("new.txt"))
        .unwrap();
    assert_eq!(diff.status, FileStatus::Renamed);
    assert!(diff.rows.is_empty());
}

#[test]
fn reads_worktree_layers() {
    let mut fixture = TestRepo::new();
    fixture.write("a.txt", "one\n");
    fixture.commit("initial");

    fixture.write("a.txt", "one\ntwo\n");
    fixture.write("untracked.txt", "fresh\n");
    fixture.write("staged.txt", "staged\n");
    {
        let mut index = fixture.repo.index().unwrap();
        index.add_path(Path::new("staged.txt")).unwrap();
        index.write().unwrap();
    }

    let repo = fixture.open();
    let snapshot = repo.worktree().unwrap();
    assert_eq!(snapshot.counts.staged, 1);
    assert_eq!(snapshot.counts.unstaged, 1);
    assert_eq!(snapshot.counts.untracked, 1);
    assert!(
        snapshot
            .files
            .iter()
            .any(|file| file.path == "a.txt" && file.layer == DiffLayer::Unstaged)
    );
    assert!(
        snapshot
            .files
            .iter()
            .any(|file| file.path == "untracked.txt" && file.layer == DiffLayer::Untracked)
    );

    let unstaged = repo
        .worktree_diff("a.txt", None, DiffLayer::Unstaged)
        .unwrap();
    assert!(
        unstaged
            .rows
            .iter()
            .any(|row| row.kind == rascal_core::DiffRowKind::Added && row.text == "two")
    );

    let untracked = repo
        .worktree_diff("untracked.txt", None, DiffLayer::Untracked)
        .unwrap();
    assert!(!untracked.rows.is_empty());

    let staged = repo
        .worktree_diff("staged.txt", None, DiffLayer::Staged)
        .unwrap();
    assert!(!staged.rows.is_empty());
}

#[test]
fn browses_tree_and_blob() {
    let mut fixture = TestRepo::new();
    fixture.write("dir/nested.txt", "alpha\nbeta\n");
    let commit = fixture.commit("files");

    let repo = fixture.open();
    let root = repo.tree_entries(commit, "").unwrap();
    assert_eq!(root.entries.len(), 1);
    assert_eq!(root.entries[0].kind, rascal_core::TreeEntryKind::Tree);

    let dir = repo.tree_entries(commit, "dir").unwrap();
    assert_eq!(dir.entries.len(), 1);
    assert_eq!(dir.entries[0].name, "nested.txt");

    let blob = repo.blob(commit, "dir/nested.txt").unwrap();
    assert!(!blob.is_binary);
    assert_eq!(blob.lines, vec!["alpha".to_string(), "beta".to_string()]);
}

#[test]
fn reports_missing_paths_and_binary_files() {
    let mut fixture = TestRepo::new();
    fixture.write_bytes("image.bin", &[0, 1, 2, 3, 0]);
    let commit = fixture.commit("binary");

    let repo = fixture.open();
    let detail = repo.commit(commit).unwrap();
    assert!(detail.files[0].is_binary);

    let diff = repo
        .commit_diff(commit, 0, None, Some("image.bin"))
        .unwrap();
    assert!(diff.is_binary);
    assert!(diff.rows.is_empty());

    assert!(GitRepo::open(&fixture.path().join("missing")).is_err());
}

#[test]
fn pages_history_and_filters_branches() {
    let mut fixture = TestRepo::new();
    fixture.write("a.txt", "0\n");
    fixture.commit("c0");
    fixture.write("a.txt", "1\n");
    fixture.commit("c1");
    fixture.write("a.txt", "2\n");
    fixture.commit("c2");
    fixture.write("a.txt", "3\n");
    fixture.commit("c3");

    let repo = fixture.open();
    let first = repo.history("HEAD", 0, 2).unwrap();
    assert_eq!(first.commits.len(), 2);
    assert!(first.has_more);
    let second = repo.history("HEAD", 2, 2).unwrap();
    assert_eq!(second.commits.len(), 2);
    assert!(!second.has_more);
    assert_ne!(first.commits[0].oid, second.commits[0].oid);

    fixture.write("branch.txt", "feature\n");
    let tip = fixture.commit("feature work");
    {
        let commit = fixture.repo.find_commit(tip).unwrap();
        fixture.repo.branch("feature", &commit, false).unwrap();
    }
    let feature = repo.history("refs/heads/feature", 0, 10).unwrap();
    assert_eq!(feature.commits[0].oid, tip);
    assert_eq!(feature.commits.len(), 5);
}

#[test]
fn reports_deleted_files() {
    let mut fixture = TestRepo::new();
    fixture.write("gone.txt", "bye\n");
    fixture.commit("add");
    std::fs::remove_file(fixture.path().join("gone.txt")).unwrap();
    let deleted = fixture.commit("delete");

    let repo = fixture.open();
    let detail = repo.commit(deleted).unwrap();
    assert_eq!(detail.files[0].status, FileStatus::Deleted);
    let diff = repo
        .commit_diff(deleted, 0, Some("gone.txt"), Some("gone.txt"))
        .unwrap();
    assert!(
        diff.rows
            .iter()
            .any(|row| row.kind == rascal_core::DiffRowKind::Removed && row.text == "bye")
    );
}

#[test]
fn handles_empty_and_bare_repositories() {
    let fixture = TestRepo::new();
    let repo = fixture.open();
    assert!(matches!(
        repo.info().unwrap().head,
        rascal_core::HeadState::Unborn
    ));
    assert!(repo.history("HEAD", 0, 10).is_err());
    assert!(repo.branches().unwrap().is_empty());

    let bare_dir = tempfile::tempdir().unwrap();
    let bare = Repository::init_bare(bare_dir.path()).unwrap();
    drop(bare);
    let bare_repo = GitRepo::open(bare_dir.path()).unwrap();
    let info = bare_repo.info().unwrap();
    assert!(info.is_bare);
    assert!(bare_repo.worktree().is_err());
}

#[test]
fn reads_file_history_and_pages_it() {
    let mut fixture = TestRepo::new();
    fixture.write("a.txt", "one\n");
    fixture.write("b.txt", "bee\n");
    let first = fixture.commit("initial");
    fixture.write("a.txt", "one\ntwo\n");
    let second = fixture.commit("edit a");
    fixture.write("b.txt", "bee\nthree\n");
    let third = fixture.commit("edit b");

    let repo = fixture.open();
    let page = repo.file_history("HEAD", "a.txt", 0, 10).unwrap();
    assert_eq!(page.commits.len(), 2);
    assert_eq!(page.commits[0].oid, second);
    assert_eq!(page.commits[1].oid, first);
    assert!(!page.has_more);

    let first_page = repo.file_history("HEAD", "a.txt", 0, 1).unwrap();
    assert_eq!(first_page.commits.len(), 1);
    assert_eq!(first_page.commits[0].oid, second);
    assert!(first_page.has_more);

    let second_page = repo.file_history("HEAD", "a.txt", 1, 1).unwrap();
    assert_eq!(second_page.commits.len(), 1);
    assert_eq!(second_page.commits[0].oid, first);
    assert!(!second_page.has_more);

    let from_third = repo
        .file_history(&third.to_string(), "b.txt", 0, 10)
        .unwrap();
    assert_eq!(from_third.commits.len(), 2);
    assert_eq!(from_third.commits[0].oid, third);
    assert_eq!(from_third.commits[1].oid, first);
}

#[test]
fn reads_one_file_diff_at_a_commit() {
    let mut fixture = TestRepo::new();
    fixture.write("a.txt", "one\n");
    fixture.write("b.txt", "bee\n");
    fixture.commit("initial");
    fixture.write("a.txt", "one\ntwo\n");
    let second = fixture.commit("edit a");
    fixture.write("b.txt", "bee\nthree\n");
    let third = fixture.commit("edit b");

    let repo = fixture.open();
    let diff = repo.commit_file_diff(second, "a.txt").unwrap().unwrap();
    assert_eq!(diff.status, FileStatus::Modified);
    assert_eq!(diff.additions, 1);
    assert!(repo.commit_file_diff(third, "a.txt").unwrap().is_none());
}

#[test]
fn reads_image_files_and_image_diffs() {
    let mut fixture = TestRepo::new();
    fixture.write_bytes("image.png", b"old-image");
    fixture.commit("add image");
    fixture.write_bytes("image.png", b"new-image");
    let second = fixture.commit("edit image");

    let repo = fixture.open();
    let image = repo.image_file(second, "image.png").unwrap();
    assert_eq!(image.path, "image.png");
    assert_eq!(image.bytes, b"new-image");
    assert!(image.oid.is_some());

    let diff = repo
        .commit_image_diff(second, None, Some("image.png"))
        .unwrap();
    assert_eq!(diff.old.as_ref().unwrap().bytes, b"old-image");
    assert_eq!(diff.new.as_ref().unwrap().bytes, b"new-image");

    fixture.write_bytes("added.png", b"added");
    let third = fixture.commit("add image");
    let diff = repo
        .commit_image_diff(third, None, Some("added.png"))
        .unwrap();
    assert!(diff.old.is_none());
    assert_eq!(diff.new.as_ref().unwrap().bytes, b"added");

    std::fs::remove_file(fixture.path().join("image.png")).unwrap();
    let fourth = fixture.commit("delete image");
    let diff = repo
        .commit_image_diff(fourth, None, Some("image.png"))
        .unwrap();
    assert_eq!(diff.old.as_ref().unwrap().bytes, b"new-image");
    assert!(diff.new.is_none());

    fixture.write_bytes("image.png", b"worktree-image");
    let diff = repo
        .worktree_image_diff("image.png", None, DiffLayer::Untracked)
        .unwrap();
    assert!(diff.old.is_none());
    assert_eq!(diff.new.as_ref().unwrap().bytes, b"worktree-image");
}
