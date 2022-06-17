// Copyright (C) 2022 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::ffi::OsStr;
use std::fs::File;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read as _;
use std::io::Result;
use std::io::Write as _;
use std::path::Path;
use std::process::ChildStdout;
use std::process::Command;
use std::process::Stdio;

use tempfile::tempdir;
use tempfile::TempDir;

use git_blamediff::await_child;
use git_blamediff::GIT;


/// The number of digits to use for representing SHA-1 check sums.
const GIT_SHA1_DIGITS: usize = 8;
/// An empty array of arguments.
const NO_ARGS: [String; 0] = [];


/// Create a `git` [`Command`].
fn git_command(directory: &Path) -> Command {
  let mut command = Command::new(GIT);
  // Because we clear the entire environment Git does not have any
  // identity and will bail out. Provide some dummy values for testing
  // purposes.
  let args = [
    "-c",
    "user.name=nobody",
    "-c",
    "user.email=nobody@example.com",
  ];

  command
    .env_clear()
    .stderr(Stdio::piped())
    .arg("-C")
    .arg(directory)
    .args(args);

  command
}

/// Execute a `git` command and wait for it to finish.
fn git<A, S>(stdout: Stdio, directory: &Path, args: A) -> Result<Option<ChildStdout>>
where
  A: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  let mut command = git_command(directory);
  let child = command
    .args(args)
    .stdin(Stdio::null())
    .stdout(stdout)
    .spawn()?;

  await_child(command.get_program(), child)
}


/// A type representing a git repositories and providing high level
/// operations on it.
struct GitRepo {
  directory: TempDir,
  commit_num: usize,
}

impl GitRepo {
  /// Create a new `git` repository in a temporary directory.
  fn new() -> Result<Self> {
    let slf = Self {
      directory: tempdir()?,
      commit_num: 0,
    };
    slf.init()?;
    Ok(slf)
  }

  /// Invoke a `git` command, ignoring any output it produces.
  fn git<A, S>(&self, args: A) -> Result<()>
  where
    A: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
  {
    git(Stdio::null(), self.directory.path(), args).map(|_| ())
  }

  /// Invoke a `git` command and capture and return its output.
  fn git_out<A, S>(&self, args: A) -> Result<Vec<u8>>
  where
    A: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
  {
    let mut output = Vec::new();
    // It is fine to unwrap here because we know that we captured stdout
    // and so it will always be available.
    let mut handle = git(Stdio::piped(), self.directory.path(), args).map(Option::unwrap)?;
    let _ = handle.read_to_end(&mut output)?;
    Ok(output)
  }

  /// Run `git init`.
  fn init(&self) -> Result<()> {
    self.git(["init"])
  }

  /// Run `git commit`.
  fn commit<A, S>(&self, args: A) -> Result<()>
  where
    A: IntoIterator<Item = S>,
    S: ToString,
  {
    let commit_num = self.commit_num.wrapping_add(1);
    let message = format!("--message=Commit #{commit_num}");
    self.git(
      ["commit".to_owned(), message]
        .into_iter()
        .chain(args.into_iter().map(|s| s.to_string())),
    )
  }

  /// Run `git add`, passing in the provided arguments.
  fn add<A, S>(&self, args: A) -> Result<()>
  where
    A: IntoIterator<Item = S>,
    S: ToString,
  {
    self.git(
      ["add"]
        .into_iter()
        .map(ToString::to_string)
        .chain(args.into_iter().map(|s| s.to_string())),
    )
  }

  /// Run `git rm`, passing in the provided arguments.
  fn remove<A, S>(&self, args: A) -> Result<()>
  where
    A: IntoIterator<Item = S>,
    S: ToString,
  {
    self.git(
      ["rm"]
        .into_iter()
        .map(ToString::to_string)
        .chain(args.into_iter().map(|s| s.to_string())),
    )
  }

  /// Run `git rev-parse`, passing in the provided arguments.
  fn rev_parse<A, S>(&self, args: A) -> Result<Vec<u8>>
  where
    A: IntoIterator<Item = S>,
    S: ToString,
  {
    self.git_out(
      ["rev-parse"]
        .into_iter()
        .map(ToString::to_string)
        .chain(args.into_iter().map(|s| s.to_string())),
    )
  }

  /// Write the provided data to a file in the repository.
  fn write<P>(&self, path: P, data: &str) -> Result<()>
  where
    P: AsRef<Path>,
  {
    let path = path.as_ref();
    if !path.is_relative() {
      return Err(Error::new(
        ErrorKind::Other,
        format!("provided path {} is not relative", path.display()),
      ))
    }

    let mut file = File::options()
      .create(true)
      .read(false)
      .write(true)
      .truncate(true)
      .open(self.directory.path().join(path))?;
    file.write_all(data.as_bytes())?;
    Ok(())
  }

  /// Invoke `git-blamediff`.
  fn blamediff<DA, DS, BA, BS>(&self, diff_args: DA, blame_args: BA) -> Result<Vec<u8>>
  where
    DA: IntoIterator<Item = DS>,
    DS: AsRef<OsStr>,
    BA: IntoIterator<Item = BS>,
    BS: AsRef<OsStr>,
  {
    let mut diff_cmd = git_command(self.directory.path());
    let mut diff_child = diff_cmd
      .args(["diff", "--relative", "--no-prefix"])
      .args(diff_args)
      .stdin(Stdio::null())
      .stdout(Stdio::piped())
      .spawn()?;

    let mut blamediff_cmd = Command::new(env!("CARGO_BIN_EXE_git-blamediff"));
    let blamediff_child = blamediff_cmd
      .current_dir(self.directory.path())
      // It is fine to unwrap here because we know that we captured
      // stdout and so it will always be available.
      .stdin(diff_child.stdout.take().unwrap())
      .stdout(Stdio::piped())
      .args(blame_args)
      .spawn()?;

    let _ = await_child(diff_cmd.get_program(), diff_child)?;
    // It is fine to unwrap here because we know that we captured
    // stdout and so it will always be available.
    let mut stdout = await_child(blamediff_cmd.get_program(), blamediff_child)?.unwrap();
    let mut output = Vec::new();
    let _ = stdout.read_to_end(&mut output)?;

    Ok(output)
  }
}


/// Check that `git-blamediff` works on a single file with a single
/// line.
#[test]
fn blame_single_file_single_line() {
  let repo = GitRepo::new().unwrap();
  repo.commit(["--allow-empty"]).unwrap();

  repo.write("main.py", "# main.py").unwrap();
  repo.add(["main.py"]).unwrap();
  repo.commit(NO_ARGS).unwrap();

  repo.write("main.py", "# main.py\n# Hello, World!").unwrap();
  let short = format!("--short={GIT_SHA1_DIGITS}");
  let sha1 = String::from_utf8(repo.rev_parse([&short, "HEAD"]).unwrap()).unwrap();
  let sha1 = sha1.trim();

  // Contrary to `git-rev-parse`, `git-blame` adds one to the provided
  // length of the SHA-1.
  let abbrev = format!("--abbrev={}", GIT_SHA1_DIGITS - 1);
  let out = repo.blamediff(NO_ARGS, [abbrev]).unwrap();
  let expected = format!(
    r#"--- main.py
+++ main.py
{sha1} 1) # main.py
"#
  );

  assert_eq!(String::from_utf8(out).unwrap(), expected);
}


/// Check that `git-blamediff` works properly on a removed file.
#[test]
fn blame_removed_file() {
  let repo = GitRepo::new().unwrap();
  repo.commit(["--allow-empty"]).unwrap();

  repo.write("main.py", "# main.py").unwrap();
  repo.add(["main.py"]).unwrap();
  repo.commit(NO_ARGS).unwrap();
  repo.remove(["main.py"]).unwrap();

  let short = format!("--short={GIT_SHA1_DIGITS}");
  let sha1 = String::from_utf8(repo.rev_parse([&short, "HEAD"]).unwrap()).unwrap();
  let sha1 = sha1.trim();

  let abbrev = format!("--abbrev={}", GIT_SHA1_DIGITS - 1);
  let out = repo.blamediff(["--staged"], [abbrev]).unwrap();
  let expected = format!(
    r#"--- main.py
+++ /dev/null
{sha1} 1) # main.py
"#
  );

  assert_eq!(String::from_utf8(out).unwrap(), expected)
}


/// Verify that we can pass additional arguments to git-blame.
#[test]
fn blame_with_additional_arguments() {
  let repo = GitRepo::new().unwrap();
  repo.commit(["--allow-empty"]).unwrap();

  repo.write("main.py", "# main.py").unwrap();
  repo.add(["main.py"]).unwrap();
  repo.commit(NO_ARGS).unwrap();

  repo.write("main.py", "# Hello, World!").unwrap();
  let sha1 = String::from_utf8(repo.rev_parse(["HEAD"]).unwrap()).unwrap();
  let sha1 = sha1.trim();

  // Tell git-blame to use the long format for SHA-1 checksums.
  let out = repo.blamediff(NO_ARGS, ["-l"]).unwrap();
  let expected = format!(
    r#"--- main.py
+++ main.py
{sha1} 1) # main.py
"#
  );

  assert_eq!(String::from_utf8(out).unwrap(), expected)
}
