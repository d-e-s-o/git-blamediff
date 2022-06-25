// Copyright (C) 2022 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::env::Args;
use std::ffi::OsStr;
use std::io::stdout;
use std::io::BufRead as _;
use std::io::BufReader;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Result;
use std::io::Write as _;
use std::ops::Deref as _;
use std::process::Child;
use std::process::ChildStdout;
use std::process::Command;
use std::process::Stdio;

use diff_parse::File;


/// The path to the `git` binary used by default.
pub const GIT: &str = "/usr/bin/git";


/// Wait for a child process to finish and map failures to an
/// appropriate error.
pub fn await_child<S>(program: S, child: Child) -> Result<Option<ChildStdout>>
where
  S: AsRef<OsStr>,
{
  let mut child = child;

  let status = child.wait()?;
  if !status.success() {
    let error = format!("process `{}` failed", program.as_ref().to_string_lossy());

    if let Some(stderr) = child.stderr {
      let mut stderr = BufReader::new(stderr);
      let mut line = String::new();

      // Let's try to include the first line of the error output in our
      // error, to at least give the user something.
      if stderr.read_line(&mut line).is_ok() {
        let line = line.trim();
        return Err(Error::new(ErrorKind::Other, format!("{error}: {line}")))
      }
    }
    return Err(Error::new(ErrorKind::Other, error))
  }
  Ok(child.stdout)
}


/// Invoke git to annotate all the diff hunks.
// TODO: For some reason `ArgsOs` is not `Clone`, which is why we pass
//       in a function that recreates such an object every time.
pub fn blame<A>(diffs: &[(File, File)], args: A) -> Result<()>
where
  A: Fn() -> Args,
{
  let out = stdout();
  let mut out = out.lock();

  for (src, dst) in diffs {
    // Start off by printing some information on the file we are
    // currently annotating.
    // TODO: We should print the file header only once.
    writeln!(out, "--- {}", src.file)?;
    writeln!(out, "+++ {}", dst.file)?;
    // Make sure stdout is flushed properly before invoking a git command
    // to be sure our output arrives before that of git.
    let () = out.flush()?;

    // Invoke git with the appropriate options to annotate the lines of
    // the diff.
    // TODO: Make the arguments here more configurable. In fact, we
    //       should not hard-code any of them here.
    let child = Command::new(GIT)
      .arg("--no-pager")
      .arg("blame")
      .arg("-s")
      .arg(format!("-L{},+{}", src.line, src.count))
      .args(args().skip(1))
      .arg("--")
      .arg(src.file.deref())
      .arg("HEAD")
      .stdin(Stdio::null())
      .stdout(Stdio::inherit())
      .stderr(Stdio::piped())
      .spawn()?;
    let _ = await_child(GIT, child)?;
  }
  Ok(())
}
