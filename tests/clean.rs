use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use harness::clean::{self, Options};
use harness::ledger::{self, Ledger};
use harness::registry::{Entry, NewEntry, new_entry};
use serde_json::json;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
}

struct Fixture {
    root: PathBuf,
    folder: PathBuf,
    ledger: PathBuf,
    entry: Entry,
}

/// A submitted contest whose folder is committed and pushed to a local bare remote.
fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("clean-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let remote = root.join("remote.git");
    let folder = root.join("Contest");
    fs::create_dir_all(folder.join("submission")).unwrap();
    git(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(&folder, &["init", "-q", "-b", "main"]);
    fs::write(folder.join(".gitignore"), "big.mp4\n").unwrap();
    fs::write(folder.join("README.md"), "# 공모전\n").unwrap();
    fs::write(folder.join("submission/final.hwp"), "hwp").unwrap();
    git(&folder, &["add", "-A"]);
    git(&folder, &["commit", "-q", "-m", "Final"]);
    git(&folder, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&folder, &["push", "-q", "-u", "origin", "main"]);

    let mut entry = new_entry(NewEntry {
        id: "x-done",
        title: "끝난 공모전",
        url: "https://e.com/done",
        source: "manual",
        organizer: "주최",
        deadline: None,
        categories: Vec::new(),
        note: None,
    });
    entry.state = "submitted".into();
    entry.folder = Some(folder.display().to_string());
    entry.session = Some(json!({"workspace": "terminal", "workspace_id": null, "runner": "omp", "handle": null}));
    entry.submissions.push(
        json!({"at": "2026-10-15T17:21+09:00", "method": "폼", "receipt": "r", "files": ["submission/final.hwp"]}),
    );
    let ledger = root.join("submissions.json");
    let item = ledger::from_submit(&entry, "폼", "r", &["submission/final.hwp".to_string()], "");
    ledger::write_at(&ledger, |l: &mut Ledger| {
        l.upsert(item);
        Ok(())
    })
    .unwrap();
    Fixture { root, folder, ledger, entry }
}

#[test]
fn pushed_submitted_contest_moves_to_trash_and_is_recorded() {
    let mut fx = fixture("ok");
    let trash = fx.root.join("trash");
    let opts = Options { trash_dir: Some(trash.clone()), ..Options::default() };
    let outcome = clean::clean(&mut fx.entry, &fx.ledger, &opts).unwrap();
    assert!(outcome.ok, "{outcome:?}");
    assert!(!fx.folder.exists());
    let moved = PathBuf::from(outcome.trash.unwrap());
    assert!(moved.starts_with(&trash) && moved.join("submission/final.hwp").exists());
    assert!(fx.entry.extra.contains_key("folder_removed_at") && fx.entry.session.is_none());
    assert!(fx.entry.history.last().unwrap().note.starts_with("로컬 폴더 정리"));
    assert_eq!(fx.entry.state, "submitted", "cleaning does not change the state");
    let item = ledger::load_at(&fx.ledger).unwrap().get("x-done").cloned().unwrap();
    assert!(item.extra["repo"].as_str().unwrap().ends_with("remote.git"));
    assert!(item.extra.contains_key("folder_removed_at"));
    assert!(!clean::candidate(&fx.entry), "an already cleaned entry is not a candidate");
    fs::remove_dir_all(&fx.root).unwrap();
}

#[test]
fn anything_that_could_lose_work_blocks_cleaning() {
    let fx = fixture("blocked");
    let ledger = ledger::load_at(&fx.ledger).unwrap();
    assert!(clean::problems(&fx.entry, &ledger).is_empty());

    fs::write(fx.folder.join("README.md"), "# 고친 내용\n").unwrap();
    let dirty = clean::problems(&fx.entry, &ledger);
    assert!(dirty.iter().any(|p| p.starts_with("커밋 안 된 변경")), "{dirty:?}");

    git(&fx.folder, &["commit", "-q", "-am", "Local only"]);
    let ahead = clean::problems(&fx.entry, &ledger);
    assert!(ahead.iter().any(|p| p.starts_with("push 안 된 커밋")), "{ahead:?}");
    git(&fx.folder, &["push", "-q"]);

    // A recorded submission file that is neither in git nor in the folder cannot be kept.
    let mut entry = fx.entry.clone();
    entry.submissions[0]["files"] = json!(["submission/final.hwp", "gone.mp4"]);
    let missing = clean::problems(&entry, &ledger);
    assert!(
        missing.iter().any(|p| p.starts_with("제출 파일이 git에도 폴더에도 없다") && p.ends_with("gone.mp4")),
        "{missing:?}"
    );

    entry.state = "running".into();
    assert!(clean::problems(&entry, &ledger).iter().any(|p| p.starts_with("상태가 running")));
    assert!(clean::problems(&fx.entry, &Ledger::default()).iter().any(|p| p.starts_with("제출 기록부에")));

    let mut blocked = entry.clone();
    let outcome = clean::clean(&mut blocked, &fx.ledger, &Options::default()).unwrap();
    assert!(!outcome.ok && fx.folder.exists(), "blocked cleaning leaves the folder");
    fs::remove_dir_all(&fx.root).unwrap();
}

#[test]
fn ignored_submission_copies_and_private_notes_are_archived_before_trashing() {
    let mut fx = fixture("archive");
    fs::write(fx.folder.join(".gitignore"), "big.mp4\n.private/\ncache/\n.DS_Store\n").unwrap();
    git(&fx.folder, &["commit", "-q", "-am", "Ignore private files"]);
    git(&fx.folder, &["push", "-q"]);
    fs::write(fx.folder.join("big.mp4"), "signed final video").unwrap(); // the signed copy kept out of git
    fs::create_dir_all(fx.folder.join(".private")).unwrap();
    fs::write(fx.folder.join(".private/form_edit_url.txt"), "https://forms.example/edit").unwrap();
    fs::create_dir_all(fx.folder.join("cache")).unwrap();
    fs::write(fx.folder.join("cache/blob.bin"), "rebuildable").unwrap();
    fs::write(fx.folder.join(".DS_Store"), "x").unwrap();
    fx.entry.submissions[0]["files"] = json!(["submission/final.hwp", "big.mp4"]);
    let ledger = ledger::load_at(&fx.ledger).unwrap();
    assert!(
        clean::problems(&fx.entry, &ledger).is_empty(),
        "an ignored but present submission copy is kept, not a blocker"
    );

    let plan = clean::archive_plan(&fx.entry, &fx.folder).unwrap();
    assert_eq!(plan.keep, vec![".private/form_edit_url.txt".to_string(), "big.mp4".into()]);
    assert_eq!(plan.dropped, 2, "the cache file and .DS_Store go with the folder, not into the archive");
    let archive = fx.root.join("archive");
    let opts =
        Options { trash_dir: Some(fx.root.join("trash")), archive_dir: Some(archive.clone()), ..Options::default() };
    let outcome = clean::clean(&mut fx.entry, &fx.ledger, &opts).unwrap();
    assert!(outcome.ok && !fx.folder.exists(), "{outcome:?}");
    let kept = archive.join("x-done");
    assert_eq!(fs::read_to_string(kept.join("big.mp4")).unwrap(), "signed final video");
    assert!(kept.join(".private/form_edit_url.txt").exists());
    assert!(!kept.join("cache").exists() && !kept.join(".DS_Store").exists() && !kept.join("README.md").exists());
    assert!(fx.entry.history.last().unwrap().note.contains("보관"));
    let item = ledger::load_at(&fx.ledger).unwrap().get("x-done").cloned().unwrap();
    assert_eq!(item.extra["archive"].as_str().unwrap(), kept.display().to_string());
    fs::remove_dir_all(&fx.root).unwrap();
}

#[test]
fn dry_run_lists_actions_without_touching_anything() {
    let mut fx = fixture("dry");
    let opts = Options { dry_run: true, trash_dir: Some(fx.root.join("trash")), ..Options::default() };
    let outcome = clean::clean(&mut fx.entry, &fx.ledger, &opts).unwrap();
    assert!(outcome.ok && fx.folder.exists() && !fx.entry.extra.contains_key("folder_removed_at"));
    assert!(outcome.actions.iter().any(|a| a.starts_with("폴더를 휴지통으로")));
    fs::remove_dir_all(&fx.root).unwrap();
}
