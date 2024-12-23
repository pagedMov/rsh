use crate::interp::{parse::Span, token::{Tk, WdFlags, WordDesc, BUILTINS, CMDSEP, KEYWORDS, REGEX, WHITESPACE}};
use libc::STDERR_FILENO;
use log::{debug,trace};
use std::{collections::VecDeque, os::fd::{AsFd, BorrowedFd}};

use super::parse::ParseErr;

pub trait StrExtension {
    fn has_unescaped(&self, check_char: char) -> bool;
}

impl StrExtension for str {
	/// Checks to see if a string slice contains a specified unescaped character. This method assumes that '\\' is the escape character.
	///
	fn has_unescaped(&self, check_char: char) -> bool {
		let mut chars = self.chars();
		let mut prev_char = None;
		chars.any(|c| {
			let check = c == check_char && prev_char != Some('\\');
			prev_char = Some(c);
			check
		})
	}
}

pub fn get_stderr<'a>() -> BorrowedFd<'a> {
	unsafe { BorrowedFd::borrow_raw(STDERR_FILENO) }
}

pub fn get_stdout<'a>() -> BorrowedFd<'a> {
	unsafe { BorrowedFd::borrow_raw(STDERR_FILENO) }
}

pub fn get_delimiter(wd: &WordDesc) -> char {
    let flags = wd.flags;
    match () {
        _ if flags.contains(WdFlags::IN_BRACE) => '}',
        _ if flags.contains(WdFlags::IN_PAREN) => ')',
        _ => unreachable!("No active delimiter found in WordDesc flags"),
    }
}
pub fn is_brace_expansion(text: &str) -> bool {
    REGEX["brace_expansion"].is_match(text) &&
		REGEX["brace_expansion"].captures(text).unwrap()[1].is_empty()
}
pub fn delimited(wd: &WordDesc) -> bool {
    wd.flags.contains(WdFlags::IN_PAREN)
}
pub fn cmdsep(c: &char) -> bool {
    CMDSEP.contains(c)
}
pub fn keywd(wd: &WordDesc) -> bool {
    KEYWORDS.contains(&wd.text.as_str()) && !wd.flags.contains(WdFlags::IS_ARG)
}
pub fn builtin(wd: &WordDesc) -> bool {
    BUILTINS.contains(&wd.text.as_str()) && !wd.flags.contains(WdFlags::IS_ARG)
}
pub fn wspace(c: &char) -> bool {
    WHITESPACE.contains(c)
}
pub fn quoted(wd: &WordDesc) -> bool {
    wd.flags.contains(WdFlags::SNG_QUOTED) || wd.flags.contains(WdFlags::DUB_QUOTED)
}
pub fn check_redirection(c: &char, chars: &mut VecDeque<char>) -> bool {
	chars.push_front(*c);
    let mut test_chars = chars.clone();
    let mut test_string = String::new();

    while let Some(c) = test_chars.pop_front() {
        if c.is_whitespace() || !matches!(c, '&' | '0'..='9' | '>' | '<') {
            break;
        }
        test_string.push(c);
    }

    if REGEX["redirection"].is_match(&test_string) {
			true
		} else {
			chars.pop_front();
			false
		}
}

pub fn process_redirection(
    word_desc: &mut WordDesc,
    chars: &mut VecDeque<char>,
) -> Result<WordDesc, ParseErr> {
    let mut redirection_text = String::new();
    while let Some(c) = chars.pop_front() {
			debug!("found this char in redirection: {}",c);
				if !matches!(c, '&' | '0'..='9' | '>' | '<') {
					chars.push_front(c);
            break;
        }
        redirection_text.push(c);
    }

		debug!("returning this word_desc text: {}",redirection_text);
    Ok(WordDesc {
        text: redirection_text,
        span: word_desc.span,
        flags: WdFlags::IS_OP,
    })
}
pub fn finalize_delimiter(word_desc: &WordDesc) -> Result<WordDesc, ParseErr> {
    let mut updated_word_desc = word_desc.clone();

    if word_desc.contains_flag(WdFlags::IN_BRACE) {
        updated_word_desc = updated_word_desc.remove_flag(WdFlags::IN_BRACE);
    } else if word_desc.contains_flag(WdFlags::IN_PAREN) {
        updated_word_desc = updated_word_desc.remove_flag(WdFlags::IN_PAREN);
    }

    Ok(updated_word_desc)
}

pub fn count_spaces(chars: &mut VecDeque<char>) -> usize {
	let mut count = 1;
	let mut buffer = VecDeque::new();
	while let Some(ch) = chars.pop_front() {
		if ch.is_whitespace() {
			buffer.push_back(ch);
			count += 1;
		} else {
			chars.push_front(ch);
			while let Some(ch) = buffer.pop_back() {
				chars.push_front(ch);
			}
			break
		}
	}
	count
}

pub fn finalize_word(word_desc: &WordDesc, tokens: &mut VecDeque<Tk>) -> Result<WordDesc,ParseErr> {
    let mut word_desc = word_desc.clone();
    let span = Span::from(word_desc.span.end,word_desc.span.end);
    trace!("finalizing word `{}` with flags `{:?}`",word_desc.text,word_desc.flags);
    if !word_desc.text.is_empty() {
        if keywd(&word_desc) {
            word_desc = word_desc.add_flag(WdFlags::KEYWORD);
        } else if builtin(&word_desc) {
            word_desc = word_desc.add_flag(WdFlags::BUILTIN);
        }
        if word_desc.flags.contains(WdFlags::EXPECT_IN) && matches!(word_desc.text.as_str(), "in") {
            debug!("setting in flag to keyword");
            word_desc = word_desc.remove_flag(WdFlags::IS_ARG);
            word_desc = word_desc.add_flag(WdFlags::KEYWORD);
        }
        tokens.push_back(Tk::from(word_desc)?);
    }

    // Always return a fresh WordDesc with reset state
    Ok(WordDesc {
        text: String::new(),
        span,
        flags: WdFlags::empty(),
    })
}
