//! Regression guard: the build script must re-run when the commit it reports moves (#8414).
//!
//! `cargo:rerun-if-env-changed` covers only the CI-supplied SHAs, so a plain commit used to leave the script stale and the binary advertising an earlier commit than the one it was built from.
//! Each case below moves the commit while touching no file inside the package, which is precisely the change an env-var-only input cannot see.

use std::fs;
use std::path::Path;
use std::process::Command;

const PACKAGE: &str = "sha-rerun-regression";

const MANIFEST: &str = "[package]\nname = \"sha-rerun-regression\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[build-dependencies]\nchrono = \"0.4\"\nwhich = \"8\"\n";

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git should be on PATH");
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_repo(root: &Path) {
    git(root, &["init", "--quiet"]);
    // The fixture commits, so it needs an identity of its own rather than whatever the machine happens to have.
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    git(root, &["config", "user.name", "Build Script Fixture"]);
}

fn commit_all(root: &Path, message: &str) {
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "-m", message]);
}

/// Write the fixture package. Its build script is the real one, copied verbatim.
fn write_package(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("Cargo.toml"), MANIFEST).unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    fs::write(root.join("build.rs"), include_str!("../build.rs")).unwrap();
    fs::write(
        root.join("build_paths.rs"),
        include_str!("../build_paths.rs"),
    )
    .unwrap();
    // The build script declares `rerun-if-changed` on this directory, so it has to exist before the first build; letting the script create it would make the second build re-run for a reason that has nothing to do with the commit, and this test would stop discriminating.
    fs::create_dir_all(root.join("static/react")).unwrap();
}

/// The commit id the build script last reported, read from the output cargo cached for it.
fn reported_sha(target: &Path) -> String {
    let build_dir = target.join("debug").join("build");
    let mut reported = Vec::new();
    for entry in fs::read_dir(&build_dir).expect("cargo should have created a build directory") {
        let entry = entry.expect("readable build directory entry");
        if !entry.file_name().to_string_lossy().starts_with(PACKAGE) {
            continue;
        }
        let Ok(text) = fs::read_to_string(entry.path().join("output")) else {
            continue;
        };
        for line in text.lines() {
            if let Some(sha) = line.strip_prefix("cargo:rustc-env=GIT_SHA=") {
                reported.push(sha.trim().to_string());
            }
        }
    }
    assert_eq!(
        reported.len(),
        1,
        "expected exactly one cached GIT_SHA for the fixture, got {reported:?}"
    );
    reported.remove(0)
}

/// Check the fixture package and return the commit id its build script reported.
fn build_and_report(package: &Path, target: &Path) -> String {
    let output = Command::new("cargo")
        .args(["check", "--quiet", "--package", PACKAGE])
        .arg("--manifest-path")
        .arg(package.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("run the build-script fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    reported_sha(target)
}

/// The build script has to answer with the commit the checkout is at, whichever one that is.
const REPORTS_CHECKOUT: &str =
    "the build script should report the commit the fixture is checked out at";

/// The second build differs from the first only by a commit that changes no file inside the package, which is exactly the move an env-var-only set of inputs cannot see.
const FOLLOWS_REPOSITORY: &str = "the build script reused the commit id it captured on its first run instead of following the repository — issue #8414";

#[test]
fn git_sha_follows_a_commit_in_the_same_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = temp.path().join("checkout");
    let target = temp.path().join("target");
    fs::create_dir_all(&checkout).unwrap();
    init_repo(&checkout);
    write_package(&checkout);
    commit_all(&checkout, "the fixture package");

    let first = git(&checkout, &["rev-parse", "--short", "HEAD"]);
    assert_eq!(
        build_and_report(&checkout, &target),
        first,
        "{REPORTS_CHECKOUT}"
    );

    // An empty commit moves the commit id without touching a single file in the package.
    git(
        &checkout,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "nothing in the package",
        ],
    );
    let second = git(&checkout, &["rev-parse", "--short", "HEAD"]);
    assert_ne!(first, second, "the fixture should be at a new commit");
    assert_eq!(
        build_and_report(&checkout, &target),
        second,
        "{FOLLOWS_REPOSITORY}"
    );
}

#[test]
fn git_sha_follows_a_commit_in_a_linked_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let checkout = temp.path().join("linked");
    let target = temp.path().join("target");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    fs::write(repo.join("README.md"), "fixture\n").unwrap();
    commit_all(&repo, "the repository");

    // A linked worktree is the layout the paths have to be resolved for: `.git` is a file there, and the branch it checks out lives in the repository's common directory rather than beside `HEAD`.
    let checkout_arg = checkout.to_str().expect("the fixture path should be UTF-8");
    git(
        &repo,
        &["worktree", "add", "--quiet", checkout_arg, "-b", "linked"],
    );

    write_package(&checkout);
    commit_all(&checkout, "the fixture package");

    let first = git(&checkout, &["rev-parse", "--short", "HEAD"]);
    assert_eq!(
        build_and_report(&checkout, &target),
        first,
        "{REPORTS_CHECKOUT}"
    );

    // `HEAD` itself holds `ref: refs/heads/linked` and is left untouched here, so this also pins down that watching it alone is not the fix.
    git(
        &checkout,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "nothing in the package",
        ],
    );
    let second = git(&checkout, &["rev-parse", "--short", "HEAD"]);
    assert_ne!(first, second, "the fixture should be at a new commit");
    assert_eq!(
        build_and_report(&checkout, &target),
        second,
        "{FOLLOWS_REPOSITORY}"
    );
}
