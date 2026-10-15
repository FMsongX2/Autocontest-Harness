//! Platform support: Linux memory readings (`/proc/meminfo`, PSI) and the crontab scheduler.
//! The real crontab is never touched; crontab I/O runs against a stub script in a temp dir.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use harness::budget;
use harness::schedule::{self, Cron};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("harness-platform-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const MEMINFO: &str = "MemTotal:       32603428 kB\nMemFree:         1048576 kB\nMemAvailable:   12582912 kB\nBuffers:          524288 kB\n";

#[test]
fn meminfo_reads_mem_available() {
    assert_eq!(budget::parse_meminfo(MEMINFO), Some(12.0));
    assert_eq!(budget::parse_meminfo("MemAvailable:    1572864 kB\n"), Some(1.5));
    // MemFree alone is not reusable memory; kernels without MemAvailable give no reading.
    assert_eq!(budget::parse_meminfo("MemTotal: 32603428 kB\nMemFree: 1048576 kB\n"), None);
    assert_eq!(budget::parse_meminfo("MemAvailable: lots kB\n"), None);
    assert_eq!(budget::parse_meminfo(""), None);
}

#[test]
fn psi_maps_some_avg10_to_pressure_levels() {
    let psi = |avg10: &str| {
        format!(
            "some avg10={avg10} avg60=0.00 avg300=0.00 total=1234\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n"
        )
    };
    assert_eq!(budget::parse_psi(&psi("0.00")), "normal");
    assert_eq!(budget::parse_psi(&psi("9.99")), "normal");
    assert_eq!(budget::parse_psi(&psi("10.00")), "warning");
    assert_eq!(budget::parse_psi(&psi("39.99")), "warning");
    assert_eq!(budget::parse_psi(&psi("40.00")), "critical");
    assert_eq!(budget::parse_psi(&psi("97.50")), "critical");
    // Only the `some` line counts, even when `full` is high.
    assert_eq!(
        budget::parse_psi("full avg10=90.00 avg60=0 avg300=0 total=0\nsome avg10=1.00 avg60=0 avg300=0 total=0\n"),
        "normal"
    );
    assert_eq!(budget::parse_psi("full avg10=90.00 avg60=0 avg300=0 total=0\n"), "unknown");
    assert_eq!(budget::parse_psi(&psi("NaN")), "unknown");
    assert_eq!(budget::parse_psi(""), "unknown");
}

fn block(root: &str, cron: &str) -> String {
    schedule::cron_block(Path::new("/opt/bin/ach"), Path::new(root), &Cron::parse(cron).unwrap(), "/usr/bin:/bin")
        .unwrap()
}

#[test]
fn cron_line_quotes_paths_and_escapes_percent() {
    let cron = Cron::parse("0  */6 * * *").unwrap();
    let line = schedule::cron_line(
        Path::new("/opt/my bin/ach"),
        Path::new("/home/a'b/50% repo"),
        &cron,
        "/usr/bin:/home/x y/bin",
    )
    .unwrap();
    assert_eq!(
        line,
        r"0 */6 * * * cd '/home/a'\''b/50\% repo' && ACH_HOME='/home/a'\''b/50\% repo' PATH='/usr/bin:/home/x y/bin' '/opt/my bin/ach' autorun >> '/home/a'\''b/50\% repo/data/logs/cron.log' 2>&1"
    );
    assert_eq!(schedule::cron_job_schedule(&line), "0 */6 * * *");
    // A newline would end the crontab entry early and run the rest as another job.
    assert!(schedule::cron_line(Path::new("/opt/harness"), Path::new("/a\nb"), &cron, "/bin").is_err());
    assert!(schedule::cron_line(Path::new("/opt/harness"), Path::new("/a"), &cron, "/bin\n* * * * * x").is_err());
}

#[test]
fn crontab_block_replace_and_remove_keep_other_lines() {
    let root = Path::new("/srv/harness");
    let user = "MAILTO=me@example.com\n# my backups\n15 3 * * * /usr/local/bin/backup --all\n";
    let other = block("/srv/other-checkout", "0 1 * * *");
    let current = format!("{user}{other}");

    let first = schedule::crontab_with(&current, root, &block("/srv/harness", "0 */6 * * *")).unwrap();
    assert!(first.starts_with(&current), "existing lines stay in place");
    assert_eq!(
        schedule::cron_job(&first, root).map(|l| schedule::cron_job_schedule(&l)).as_deref(),
        Some("0 */6 * * *")
    );
    // Idempotent: installing the same block again changes nothing.
    assert_eq!(schedule::crontab_with(&first, root, &block("/srv/harness", "0 */6 * * *")).unwrap(), first);

    // Updating replaces this checkout's block only; there is still exactly one.
    let second = schedule::crontab_with(&first, root, &block("/srv/harness", "30 2 * * 1")).unwrap();
    assert_eq!(second.matches("# BEGIN autocontest-harness /srv/harness\n").count(), 1);
    assert_eq!(
        schedule::cron_job(&second, root).map(|l| schedule::cron_job_schedule(&l)).as_deref(),
        Some("30 2 * * 1")
    );
    assert!(!second.contains("0 */6 * * *"));
    assert!(second.contains(&other) && second.starts_with(user));

    // Removing leaves exactly what was there before, including the other checkout's block.
    let removed = schedule::crontab_without(&second, root).unwrap();
    assert_eq!(removed, current);
    assert_eq!(schedule::cron_job(&removed, root), None);
    assert_eq!(schedule::crontab_without(&removed, root).unwrap(), removed, "removing twice is a no-op");
    assert!(schedule::cron_job(&removed, Path::new("/srv/other-checkout")).is_some());

    // A root that is a prefix of another root's path does not match its block.
    let prefixed =
        schedule::crontab_with("", Path::new("/srv/harness2"), &block("/srv/harness2", "0 1 * * *")).unwrap();
    assert_eq!(schedule::cron_job(&prefixed, root), None);
    assert_eq!(schedule::crontab_without(&prefixed, root).unwrap(), prefixed);
}

#[test]
fn crontab_without_trailing_newline_and_unterminated_block() {
    let root = Path::new("/srv/harness");
    let joined = schedule::crontab_with("@reboot /bin/true", root, &block("/srv/harness", "0 1 * * *")).unwrap();
    assert!(joined.starts_with("@reboot /bin/true\n# BEGIN autocontest-harness /srv/harness\n"));
    // A begin marker with no end must not swallow the user's lines below it.
    let broken = "# BEGIN autocontest-harness /srv/harness\n0 1 * * * x\n15 3 * * * /usr/local/bin/backup\n";
    assert!(schedule::crontab_without(broken, root).is_err());
    assert!(schedule::crontab_with(broken, root, &block("/srv/harness", "0 1 * * *")).is_err());
}

#[test]
fn a_user_without_a_crontab_is_an_empty_table_on_every_cron_flavour() {
    assert!(schedule::crontab_missing("crontab: no crontab for alice\n"));
    assert!(schedule::crontab_missing("crontab: can't open 'root': No such file or directory\n"));
    assert!(!schedule::crontab_missing("crontab: Permission denied\n"));
    assert!(!schedule::crontab_missing("crontab: can't open 'root': Permission denied\n"));
}

/// A `crontab` stand-in that keeps its table in `<dir>/table` (`-l` fails like the real one when
/// there is none) and records every write.
fn stub_crontab(dir: &Path) -> String {
    let script = dir.join("crontab");
    let table = dir.join("table");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$1\" = -l ]; then\n  [ -f '{t}' ] || {{ echo \"crontab: no crontab for tester\" >&2; exit 1; }}\n  cat '{t}'\nelif [ \"$1\" = - ]; then\n  cat > '{t}'\n  echo write >> '{d}/writes'\nelse\n  exit 2\nfi\n",
            t = table.display(),
            d = dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script.display().to_string()
}

#[test]
fn cron_install_update_remove_through_crontab_command() {
    let dir = scratch("crontab");
    let program = stub_crontab(&dir);
    let root = dir.join("checkout");
    let exe = Path::new("/opt/bin/ach");
    let table = dir.join("table");

    // No crontab yet reads as empty.
    assert_eq!(schedule::read_crontab(&program).unwrap(), "");
    let created =
        schedule::cron_install_with(&program, exe, &root, &Cron::parse("0 */6 * * *").unwrap(), "/bin").unwrap();
    assert_eq!(created["action"], "created");
    assert!(root.join("data/logs").is_dir(), "the cron log folder exists before the first run");

    std::fs::write(&table, format!("MAILTO=me\n{}", std::fs::read_to_string(&table).unwrap())).unwrap();
    let updated =
        schedule::cron_install_with(&program, exe, &root, &Cron::parse("5 1 * * *").unwrap(), "/bin").unwrap();
    assert_eq!(updated["action"], "updated");
    let text = std::fs::read_to_string(&table).unwrap();
    assert!(text.starts_with("MAILTO=me\n"));
    assert_eq!(schedule::cron_job(&text, &root).map(|l| schedule::cron_job_schedule(&l)).as_deref(), Some("5 1 * * *"));

    assert_eq!(schedule::cron_remove_with(&program, &root).unwrap()["removed"], true);
    assert_eq!(std::fs::read_to_string(&table).unwrap(), "MAILTO=me\n");
    // Nothing to remove: no write at all.
    let writes = std::fs::read_to_string(dir.join("writes")).unwrap().lines().count();
    assert_eq!(schedule::cron_remove_with(&program, &root).unwrap()["removed"], false);
    assert_eq!(std::fs::read_to_string(dir.join("writes")).unwrap().lines().count(), writes);

    // Any other `crontab -l` failure is an error, not an empty table to overwrite.
    std::fs::write(dir.join("crontab"), "#!/bin/sh\necho 'crontab: permission denied' >&2\nexit 1\n").unwrap();
    assert!(schedule::read_crontab(&program).is_err());
    assert!(schedule::cron_install_with(&program, exe, &root, &Cron::parse("0 1 * * *").unwrap(), "/bin").is_err());
    let missing = dir.join("no-such-crontab").display().to_string();
    assert!(schedule::read_crontab(&missing).is_err());
}
