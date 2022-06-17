// Copyright (C) 2022 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::env::args;
use std::io::stdin;
use std::io::Result;

use git_blamediff::blame;
use git_blamediff::diff::Parser;


/// Parse the diff from stdin and invoke git blame on each hunk.
fn main() -> Result<()> {
  let mut parser = Parser::new();
  parser.parse(stdin().lock())?;

  // TODO: We may want to catch BrokenPipe errors here and exit
  //       gracefully.
  blame(parser.diffs(), args)
}
