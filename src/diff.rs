// Copyright (C) 2022 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! A module for parsing diffs.

use std::io::BufRead;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Result as IoResult;
use std::rc::Rc;
use std::str::FromStr;

use once_cell::sync::Lazy;

use regex::Regex;

const WS_STRING: &str = r"[ \t]*";
const FILE_STRING: &str = r"([^ \t]+)";
const ADDSUB_STRING: &str = r"([+\-])";
const NUMLINE_STRING: &str = r"([0-9]+)";

static DIFF_DIFF_REGEX: Lazy<Regex> = Lazy::new(|| {
  // Aside from '+' and '-' we have a "continuation" character ('\') in
  // here which essentially just indicates a line that is being ignored.
  // This character is used (in conjunction with the string "No newline at
  // end of file") to indicate that a newline symbol at the end of a file
  // is added or removed, for instance.
  Regex::new(r"^[+\-\\ ]").unwrap()
});
static DIFF_NODIFF_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[^+\- ]").unwrap());
static DIFF_SRC_REGEX: Lazy<Regex> =
  Lazy::new(|| Regex::new(&format!("^---{WS_STRING}{FILE_STRING}")).unwrap());
static DIFF_DST_REGEX: Lazy<Regex> =
  Lazy::new(|| Regex::new(&format!(r"^\+\+\+{WS_STRING}{FILE_STRING}")).unwrap());
static DIFF_HEAD_REGEX: Lazy<Regex> = Lazy::new(|| {
  // Note that in case a new file containing a single line is added the
  // diff header might not contain the second count.
  Regex::new(&format!(
    "^@@ {ADDSUB_STRING}{NUMLINE_STRING}(?:,{NUMLINE_STRING})? \
         {ADDSUB_STRING}{NUMLINE_STRING}(?:,{NUMLINE_STRING})? @@"
  ))
  .unwrap()
});


/// An enumeration of the supported operations in a diff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
  /// Lines are being added.
  Add,
  /// Lines are being removed.
  Sub,
}

impl FromStr for Op {
  type Err = ();

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s {
      "+" => Ok(Self::Add),
      "-" => Ok(Self::Sub),
      _ => Err(()),
    }
  }
}


/// An object capturing meta data about a diff.
#[derive(Debug)]
pub struct File {
  /// The file the diff belongs to.
  pub file: Rc<String>,
  /// Whether the diff adds or removes lines.
  pub op: Op,
  /// The start line of the diff.
  pub line: usize,
  /// The number of lines in the diff.
  pub count: usize,
}


/// An enumeration of all the states our parser can be in.
#[derive(Clone, Debug)]
enum State {
  /// The state when we expect a new file to start.
  Start,
  /// The state after we parsed the source file header part.
  Src { src: Rc<String> },
  /// The state after we parsed the destination file header part.
  Dst { src: Rc<String>, dst: Rc<String> },
  /// The state after we parsed the entire header.
  Hdr { src: Rc<String>, dst: Rc<String> },
}

impl State {
  /// A helper function advancing `self` to another state.
  fn advance(&mut self, state: State) -> Option<IoResult<()>> {
    *self = state;
    Some(Ok(()))
  }

  /// Try parsing a line containing information about the changed lines.
  fn parse_head(
    &mut self,
    diffs: &mut Vec<(File, File)>,
    line: &str,
    src: Rc<String>,
    dst: Rc<String>,
  ) -> Option<IoResult<()>> {
    let captures = DIFF_HEAD_REGEX.captures(line)?;

    let mut parse = || -> IoResult<()> {
      // It is fine to unwrap captures 1-2 and 4-5 because we know they
      // participate in the match unconditionally.
      let add_src = captures.get(1).unwrap().as_str();
      let start_src = captures.get(2).unwrap().as_str();
      // Because a diff header might not contain counts if only a single
      // line is affected, we provide the default "1" here.
      let count_src = captures.get(3).map(|m| m.as_str()).unwrap_or("1");
      let add_dst = captures.get(4).unwrap().as_str();
      let start_dst = captures.get(5).unwrap().as_str();
      let count_dst = captures.get(6).map(|m| m.as_str()).unwrap_or("1");

      let src_file = File {
        file: src.clone(),
        // It is fine to unwrap here because the regex would not have
        // matched if the operation was not valid.
        op: add_src.parse().unwrap(),
        line: start_src.parse().map_err(|error| {
          Error::new(
            ErrorKind::Other,
            format!(r#"failed to parse start line number in line: "{line}": {error}"#),
          )
        })?,
        count: count_src.parse().map_err(|error| {
          Error::new(
            ErrorKind::Other,
            format!(r#"failed to parse line count in line: "{line}": {error}"#),
          )
        })?,
      };
      let dst_file = File {
        file: dst.clone(),
        // It is fine to unwrap here because the regex would not have
        // matched if the operation was not valid.
        op: add_dst.parse().unwrap(),
        line: start_dst.parse().map_err(|error| {
          Error::new(
            ErrorKind::Other,
            format!(r#"failed to parse start line number in line: "{line}": {error}"#),
          )
        })?,
        count: count_dst.parse().map_err(|error| {
          Error::new(
            ErrorKind::Other,
            format!(r#"failed to parse line count in line: "{line}": {error}"#),
          )
        })?,
      };
      diffs.push((src_file, dst_file));
      Ok(())
    };


    if let Err(error) = parse() {
      return Some(Err(error))
    }
    self.advance(Self::Hdr { src, dst })
  }

  /// Try parsing a line containing the source file.
  fn parse_src(&mut self, line: &str) -> Option<IoResult<()>> {
    let captures = DIFF_SRC_REGEX.captures(line)?;
    // It is fine to unwrap here because we know the queried capture
    // group participates in the match unconditionally.
    let src = captures.get(1).unwrap();

    self.advance(Self::Src {
      src: Rc::new(src.as_str().to_owned()),
    })
  }

  /// Try parsing a line containing the destination file.
  fn parse_dst(&mut self, line: &str, src: Rc<String>) -> Option<IoResult<()>> {
    let captures = DIFF_DST_REGEX.captures(line)?;
    // It is fine to unwrap here because we know the queried capture
    // group participates in the match unconditionally.
    let dst = captures.get(1).unwrap();

    self.advance(Self::Dst {
      src,
      dst: Rc::new(dst.as_str().to_owned()),
    })
  }

  /// Try matching a line that contains no actual diff.
  fn match_no_diff(&mut self, line: &str) -> Option<IoResult<()>> {
    DIFF_NODIFF_REGEX.is_match(line).then(|| Ok(()))
  }

  /// Try matching an actual diff line.
  fn match_diff(&mut self, line: &str) -> Option<IoResult<()>> {
    DIFF_DIFF_REGEX.is_match(line).then(|| Ok(()))
  }

  /// Try matching a line not from an actual diff that indicates the
  /// start of a new file.
  fn restart(&mut self, line: &str) -> Option<IoResult<()>> {
    DIFF_NODIFF_REGEX.is_match(line).then(|| ())?;
    self.advance(Self::Start)
  }

  fn parse(&mut self, diffs: &mut Vec<(File, File)>, line: &str) -> IoResult<()> {
    /// Check and evaluate the result of a parser function.
    macro_rules! check {
      ($result:expr) => {
        match $result {
          // The parser did not match. Continue with the next one.
          None => (),
          // The parser matched and then either continued parsing
          // successfully or produced an error. Short circuit in both cases
          // to bubble up the result.
          Some(result) => return result,
        }
      };
    }

    // This clone is a mere bump of two `Rc` counts, at most.
    match self.clone() {
      State::Start => {
        check!(self.parse_src(line));
        check!(self.match_no_diff(line));
      },
      State::Src { src } => {
        check!(self.parse_dst(line, src));
      },
      State::Dst { src, dst } => {
        check!(self.parse_head(diffs, line, src, dst));
      },
      State::Hdr { src, dst } => {
        check!(self.match_diff(line));
        check!(self.parse_head(diffs, line, src, dst));
        check!(self.restart(line));
      },
    };

    Err(Error::new(
      ErrorKind::Other,
      format!(r#"encountered unexpected line: "{line}" (state: {self:?})"#),
    ))
  }
}


/// A type interpreting a diff and extracting relevant information.
pub struct Parser {
  state: State,
  diffs: Vec<(File, File)>,
}

impl Parser {
  /// Create a new `Parser` object in its initial state.
  #[inline]
  pub fn new() -> Self {
    Self {
      state: State::Start,
      diffs: Vec::new(),
    }
  }

  /// Parse a list of lines.
  pub fn parse<L>(&mut self, mut lines: L) -> IoResult<()>
  where
    L: BufRead,
  {
    let mut line = String::new();

    loop {
      line.clear();

      let count = lines.read_line(&mut line)?;
      if count == 0 {
        // We have reached end-of-file.
        break Ok(())
      }

      // Remove trailing new line symbols, we already expect lines.
      let line = if let Some(line) = line.strip_suffix('\n') {
        line
      } else {
        &line
      };
      // We simply ignore any empty lines and do not even hand them into
      // the state for further consideration because they cannot change
      // anything.
      if !line.is_empty() {
        let () = self.state.parse(&mut self.diffs, line)?;
      }
    }
  }

  /// Retrieve all found diffs.
  pub fn diffs(&self) -> &[(File, File)] {
    &self.diffs
  }
}


#[cfg(test)]
mod tests {
  use super::*;

  use std::ops::Deref as _;


  /// Test parsing of a very simple one-line-change diff.
  #[test]
  fn parse_simple_diff() {
    let diff = r#"
--- main.c
+++ main.c
@@ -6,6 +6,6 @@ int main(int argc, char const* argv[])
     fprintf(stderr, "Too many arguments.\n");
     return -1;
   }
-  printf("Hello world!");
+  printf("Hello world!\n");
   return 0;
 }"#;

    let mut parser = Parser::new();
    let () = parser.parse(diff.as_bytes()).unwrap();

    let diffs = parser.diffs();
    assert_eq!(diffs.len(), 1);

    let (src, dst) = &diffs[0];
    assert_eq!(src.file.deref(), "main.c");
    assert_eq!(src.op, Op::Sub);
    assert_eq!(src.line, 6);
    assert_eq!(src.count, 6);

    assert_eq!(dst.file.deref(), "main.c");
    assert_eq!(dst.op, Op::Add);
    assert_eq!(dst.line, 6);
    assert_eq!(dst.count, 6);
  }

  /// Test that we can parse a diff emitted by git if a file's trailing
  /// newline is added.
  #[test]
  fn parse_diff_adding_newline_at_end_of_file() {
    let diff = r#"
--- main.c
+++ main.c
@@ -8,4 +8,4 @@ int main(int argc, char const* argv[])
   }
   printf("Hello world!");
   return 0;
-}
\\ No newline at end of file
+}"#;

    let mut parser = Parser::new();
    let () = parser.parse(diff.as_bytes()).unwrap();

    let diffs = parser.diffs();
    assert_eq!(diffs.len(), 1);

    let (src, dst) = &diffs[0];
    assert_eq!(src.file.deref(), "main.c");
    assert_eq!(src.op, Op::Sub);
    assert_eq!(src.line, 8);
    assert_eq!(src.count, 4);

    assert_eq!(dst.file.deref(), "main.c");
    assert_eq!(dst.op, Op::Add);
    assert_eq!(dst.line, 8);
    assert_eq!(dst.count, 4);
  }

  /// Test that we can parse a diff emitted by git if a file's trailing
  /// newline is removed.
  #[test]
  fn parse_diff_removing_newline_at_end_of_file() {
    let diff = r#"
--- main.c
+++ main.c
@@ -8,4 +8,4 @@ int main(int argc, char const* argv[])
   }
   printf("Hello world!");
   return 0;
-}
+}
\\ No newline at end of file"#;

    let mut parser = Parser::new();
    let () = parser.parse(diff.as_bytes()).unwrap();

    let diffs = parser.diffs();
    assert_eq!(diffs.len(), 1);

    let (src, dst) = &diffs[0];
    assert_eq!(src.file.deref(), "main.c");
    assert_eq!(src.op, Op::Sub);
    assert_eq!(src.line, 8);
    assert_eq!(src.count, 4);

    assert_eq!(dst.file.deref(), "main.c");
    assert_eq!(dst.op, Op::Add);
    assert_eq!(dst.line, 8);
    assert_eq!(dst.count, 4);
  }

  /// Test that we can parse a diff adding a file with a single line."""
  #[test]
  fn parse_diff_with_added_file_with_single_line() {
    let diff = r#"
--- /dev/null
+++ main.c
@@ -0,0 +1 @@
+main.c"#;

    let mut parser = Parser::new();
    let () = parser.parse(diff.as_bytes()).unwrap();

    let diffs = parser.diffs();
    assert_eq!(diffs.len(), 1);

    let (src, dst) = &diffs[0];
    assert_eq!(src.file.deref(), "/dev/null");
    assert_eq!(src.op, Op::Sub);
    assert_eq!(src.line, 0);
    assert_eq!(src.count, 0);

    assert_eq!(dst.file.deref(), "main.c");
    assert_eq!(dst.op, Op::Add);
    assert_eq!(dst.line, 1);
    assert_eq!(dst.count, 1);
  }

  /// Test that we can parse a diff removing a file with a single line.
  #[test]
  fn parse_diff_with_removed_file_with_single_line() {
    let diff = r#"
--- main.c
+++ /dev/null
@@ -1 +0,0 @@
-main.c"#;

    let mut parser = Parser::new();
    let () = parser.parse(diff.as_bytes()).unwrap();

    let diffs = parser.diffs();
    assert_eq!(diffs.len(), 1);

    let (src, dst) = &diffs[0];
    assert_eq!(src.file.deref(), "main.c");
    assert_eq!(src.op, Op::Sub);
    assert_eq!(src.line, 1);
    assert_eq!(src.count, 1);

    assert_eq!(dst.file.deref(), "/dev/null");
    assert_eq!(dst.op, Op::Add);
    assert_eq!(dst.line, 0);
    assert_eq!(dst.count, 0);
  }

  /// Verify that we can parse a diff containing an empty line.
  #[test]
  fn parse_diff_with_empty_line() {
    let diff = r#"
--- main.c
+++ main.c
@@ -1,6 +1,6 @@
 #include <stdio.h>
 
-int main(int argc, char const* argv[])
+int main(int argc, char* argv[])
 {
   if (argc > 1) {
     fprintf(stderr, "Too many arguments.\n");"#;

    let mut parser = Parser::new();
    let () = parser.parse(diff.as_bytes()).unwrap();

    let diffs = parser.diffs();
    assert_eq!(diffs.len(), 1);

    let (src, dst) = &diffs[0];
    assert_eq!(src.file.deref(), "main.c");
    assert_eq!(src.op, Op::Sub);
    assert_eq!(src.line, 1);
    assert_eq!(src.count, 6);

    assert_eq!(dst.file.deref(), "main.c");
    assert_eq!(dst.op, Op::Add);
    assert_eq!(dst.line, 1);
    assert_eq!(dst.count, 6);
  }
}
