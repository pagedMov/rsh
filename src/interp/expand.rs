use chrono::Local;
use glob::glob;
use regex::Regex;
use std::collections::VecDeque;
use std::io::{BufReader, Read};
use std::mem::take;
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::PathBuf;
use crate::event::ShError;
use crate::execute::{self, ProcIO, RustFd};
use crate::interp::parse::NdFlags;
use crate::interp::token::{Tk, TkType, WdFlags, WordDesc};
use crate::interp::helper::{self,StrExtension, VecDequeExtension};
use crate::shellenv::{read_logic, read_meta, read_vars, write_meta, EnvFlags, SavedEnv};
use crate::RshResult;

use super::parse::{self, NdType, Node, ParseState, Span};
use super::token::RshTokenizer;

pub fn check_globs(string: String) -> bool {
	string.has_unescaped("?") ||
		string.has_unescaped("*")
}

pub fn expand_shebang(mut body: String) -> RshResult<String> {
	// If no shebang, use the path to rsh
	// and attach the '--subshell' argument to signal to the rsh subprocess that it is in a subshell context
	if !body.starts_with("#!") {
		let interpreter = std::env::current_exe().unwrap();
		let mut shebang = "#!".to_string();
		shebang.push_str(interpreter.to_str().unwrap());
		shebang = format!("{} {}", shebang, "--subshell");
		shebang.push('\n');
		shebang.push_str(&body);
		return Ok(shebang);
	}

	// If there is an abbreviated shebang (e.g. `#!python`), find the path to the interpreter using the PATH env var, and expand the command name to the full path (e.g. `#!python` -> `#!/usr/bin/python`)
	if body.starts_with("#!") && !body.lines().next().unwrap_or_default().contains('/') {
		let mut command = String::new();
		let mut body_chars = body.chars().collect::<VecDeque<char>>();
		body_chars.pop_front(); body_chars.pop_front();

		while let Some(ch) = body_chars.pop_front() {
			if matches!(ch, ' ' | '\t' | '\n' | ';') {
				while body_chars.front().is_some_and(|ch| matches!(ch, ' ' | '\t' | '\n' | ';')) {
					body_chars.pop_front();
				}
				body = body_chars.iter().collect::<String>();
				break;
			} else {
				command.push(ch);
			}
		}
		if let Some(path) = helper::which(&command) {
			let path = format!("{}{}{}", "#!", path, '\n');
			return Ok(format!("{}{}", path, body));
		}
	}

	Ok(body)
}

pub fn expand_arguments(node: &mut Node) -> RshResult<Vec<Tk>> {
	let argv = node.get_argv()?;
	let mut cmd_name = None;
	let mut glob = true;
	let mut expand_buffer = Vec::new();
	for arg in &argv {
		if cmd_name.is_none() {
			cmd_name = Some(arg.text());
			if cmd_name == Some("expr") { // We don't expand globs for the `expr` builtin
				glob = false;
			}
			expand_buffer.push(arg.clone()); // We also don't expand command names
			continue
		}
		let mut expanded = expand_token(arg.clone(),glob)?;
		while let Some(token) = expanded.pop_front() {
			if !token.text().is_empty() {
				// Do not return empty tokens
				expand_buffer.push(token);
			}
		}
	}
	match &node.nd_type {
		NdType::Builtin {..} => {
			node.nd_type = NdType::Builtin { argv: expand_buffer.clone().into() };
			Ok(expand_buffer)
		}
		NdType::Command {..}  => {
			node.nd_type = NdType::Command { argv: expand_buffer.clone().into() };
			Ok(expand_buffer)
		}
		NdType::Subshell { body, argv: _ } => {
			node.nd_type = NdType::Subshell { body: body.to_string(), argv: expand_buffer.clone().into() };
			Ok(expand_buffer)
		}
		_ => Err(ShError::from_internal("Called expand arguments on a non-command node"))
	}
}

pub fn esc_seq(c: char) -> Option<char> {
	//TODO:
	match c {
		'a' => Some('\x07'),
		'n' => Some('\n'),
		't' => Some('\t'),
		'\\' => Some('\\'),
		'"' => Some('"'),
		'\'' => Some('\''),
		_ => panic!()
	}
}

pub fn expand_time(fmt: &str) -> String {
	let right_here_right_now = Local::now();
	right_here_right_now.format(fmt).to_string()
}

pub fn expand_prompt() -> RshResult<String> {
	// TODO:
	//\j - number of managed jobs
	//\l - shell's terminal device name
	//\v - rsh version
	//\V - rsh release; version + patch level
	//\! - history number of this command
	//\# - command number of this command
	let default_color = if read_vars(|vars| vars.get_evar("UID").is_some_and(|uid| uid == "0"))? {
		"31" // Red if uid is 0, aka root user
	} else {
		"32" // Green if anyone else
	};
	let cwd: String = read_vars(|vars| vars.get_evar("PWD").map_or("".into(), |cwd| cwd).to_string())?;
	let default_path = if cwd.as_str() == "/" {
		"\\e[36m\\w\\e[0m".to_string()
	} else {
		format!("\\e[1;{}m\\w\\e[1;36m/\\e[0m",default_color)
	};
	let ps1: String = read_vars(|vars| vars.get_evar("PS1").map_or(format!("\\n{}\\n\\e[{}mdebug \\$\\e[36m>\\e[0m ",default_path,default_color), |ps1| ps1.clone()))?;
	let mut result = String::new();
	let mut chars = ps1.chars().collect::<VecDeque<char>>();
	while let Some(c) = chars.pop_front() {
		match c {
			'\\' => {
				if let Some(esc_c) = chars.pop_front() {
					match esc_c {
						'a' => result.push('\x07'),
						'n' => result.push('\n'),
						'r' => result.push('\r'),
						'\\' => result.push('\\'),
						'\'' => result.push('\''),
						'"' => result.push('"'),
						'd' => result.push_str(expand_time("%a %b %d").as_str()),
						't' => result.push_str(expand_time("%H:%M:%S").as_str()),
						'T' => result.push_str(expand_time("%I:%M:%S").as_str()),
						'A' => result.push_str(expand_time("%H:%M").as_str()),
						'@' => result.push_str(expand_time("%I:%M %p").as_str()),
						_ if esc_c.is_digit(8) => {
							let mut octal_digits = String::new();
							octal_digits.push(esc_c); // Add the first digit

							for _ in 0..2 {
								if let Some(next_c) = chars.front() {
									if next_c.is_digit(8) {
										octal_digits.push(chars.pop_front().unwrap());
									} else {
										break;
									}
								}
							}

							if let Ok(value) = u8::from_str_radix(&octal_digits, 8) {
								result.push(value as char);
							} else {
								// Invalid sequence, treat as literal
								result.push_str(&format!("\\{}", octal_digits));
							}
						}
						'e' => {
							result.push('\x1B');
							if chars.front().is_some_and(|&ch| ch == '[') {
								result.push(chars.pop_front().unwrap()); // Consume '['
								while let Some(ch) = chars.pop_front() {
									result.push(ch);
									if ch == 'm' {
										break; // End of ANSI sequence
									}
								}
							}
						}
						'[' => {
							// Handle \[ (start of non-printing sequence)
							while let Some(ch) = chars.pop_front() {
								if ch == ']' {
									break; // Stop at the closing \]
								}
								result.push(ch); // Add non-printing content
							}
						}
						']' => {
							// Handle \] (end of non-printing sequence)
							// Do nothing, it's just a marker
						}
						'w' => {
							let mut cwd = read_vars(|vars| vars.get_evar("PWD").map_or(String::new(), |pwd| pwd.to_string()))?;
							let home = read_vars(|vars| vars.get_evar("HOME").map_or("".into(), |home| home))?;
							if cwd.starts_with(&home) {
								cwd = cwd.replacen(&home, "~", 1); // Use `replacen` to replace only the first occurrence
							}
							// TODO: unwrap is probably safe here since this option is initialized with the environment but it might still cause issues later if this is left unhandled
							let trunc_len = read_meta(|meta| meta.get_shopt("trunc_prompt_path").unwrap_or(0))?;
							if trunc_len > 0 {
								let mut path = PathBuf::from(cwd);
								let mut cwd_components: Vec<_> = path.components().collect();
								if cwd_components.len() > trunc_len {
									cwd_components = cwd_components.split_off(cwd_components.len() - trunc_len);
									path = cwd_components.iter().collect(); // Rebuild the PathBuf
								}
								cwd = path.to_string_lossy().to_string();
							}
							result.push_str(&cwd);
						}
						'W' => {
							let cwd = PathBuf::from(read_vars(|vars| vars.get_evar("PWD").map_or("".to_string(), |pwd| pwd.to_string()))?);
							let mut cwd = cwd.components().last().map(|comp| comp.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
							let home = read_vars(|vars| vars.get_evar("HOME").map_or("".into(), |home| home))?;
							if cwd.starts_with(&home) {
								cwd = cwd.replacen(&home, "~", 1); // Replace HOME with '~'
							}
							result.push_str(&cwd);
						}
						'H' => {
							let hostname: String = read_vars(|vars| vars.get_evar("HOSTNAME").map_or("unknown host".into(), |host| host))?;
							result.push_str(&hostname);
						}
						'h' => {
							let hostname = read_vars(|vars| vars.get_evar("HOSTNAME").map_or("unknown host".into(), |host| host))?;
							if let Some((hostname, _)) = hostname.split_once('.') {
								result.push_str(hostname);
							} else {
								result.push_str(&hostname); // No '.' found, use the full hostname
							}
						}
						's' => {
							let sh_name = read_vars(|vars| vars.get_evar("SHELL").map_or("rsh".into(), |sh| sh))?;
							result.push_str(&sh_name);
						}
						'u' => {
							let user = read_vars(|vars| vars.get_evar("USER").map_or("unknown".into(), |user| user))?;
							result.push_str(&user);
						}
						'$' => {
							let uid = read_vars(|vars| vars.get_evar("UID").map_or("0".into(), |uid| uid))?;
							match uid.as_str() {
								"0" => result.push('#'),
								_ => result.push('$'),
							}
						}
						_ => {
							result.push('\\');
							result.push(esc_c);
						}
					}
				} else {
					result.push('\\');
				}
			}
			_ => result.push(c)
		}
	}
	Ok(result)
}

pub fn process_ansi_escapes(input: &str) -> String {
	let mut result = String::new();
	let mut chars = input.chars().collect::<VecDeque<char>>();

	while let Some(c) = chars.pop_front() {
		if c == '\\' {
			if let Some(next) = chars.pop_front() {
				match next {
					'a' => result.push('\x07'), // Bell
					'b' => result.push('\x08'), // Backspace
					't' => result.push('\t'),   // Tab
					'n' => result.push('\n'),   // Newline
					'r' => result.push('\r'),   // Carriage return
					'e' | 'E' => result.push('\x1B'), // Escape (\033 in octal)
					'0' => {
						// Octal escape: \0 followed by up to 3 octal digits
						let mut octal_digits = String::new();
						while octal_digits.len() < 3 && chars.front().is_some_and(|ch| ch.is_digit(8)) {
							octal_digits.push(chars.pop_front().unwrap());
						}
						if let Ok(value) = u8::from_str_radix(&octal_digits, 8) {
							let character = value as char;
							result.push(character);
							// Check for ANSI sequence if the result is ESC (\033 or \x1B)
							if character == '\x1B' && chars.front().is_some_and(|&ch| ch == '[') {
								result.push(chars.pop_front().unwrap()); // Consume '['
								while let Some(ch) = chars.pop_front() {
									result.push(ch);
									if ch == 'm' {
										break; // Stop at the end of the ANSI sequence
									}
								}
							}
						}
					}
					_ => {
						// Unknown escape, treat literally
						result.push('\\');
						result.push(next);
					}
				}
			} else {
				// Trailing backslash, treat literally
				result.push('\\');
			}
		} else if c == '\x1B' {
			// Handle raw ESC characters (e.g., \033 in octal or actual ESC char)
			result.push(c);
			if chars.front().is_some_and(|&ch| ch == '[') {
				result.push(chars.pop_front().unwrap()); // Consume '['
				while let Some(ch) = chars.pop_front() {
					result.push(ch);
					if ch == 'm' {
						break; // Stop at the end of the ANSI sequence
					}
				}
			}
		} else {
			result.push(c);
		}
	}

	result
}

pub fn expand_alias(alias: &str) -> RshResult<String> {
	if let Some(alias_content) = read_logic(|log| log.get_alias(alias))? {
		Ok(alias_content)
	} else {
		Err(ShError::from_internal(format!("Did not find an alias for this: {}",alias).as_str()))
	}
}

pub fn check_home_expansion(text: &str) -> bool {
	text.has_unescaped("~") && (
		text.starts_with('~') ||
		text.has_unescaped("~/")
	)
}

pub fn expand_token(token: Tk, expand_glob: bool) -> RshResult<VecDeque<Tk>> {
	let mut working_buffer: VecDeque<Tk> = VecDeque::new();
	let mut product_buffer: VecDeque<Tk> = VecDeque::new();
	let split_words = token.tk_type != TkType::String;

	//TODO: find some way to clean up this surprisingly functional mess
	// Escaping breaks this right now I think

	working_buffer.push_back(token.clone());
	while let Some(mut token) = working_buffer.pop_front() {
		// If expand_glob is true, then check for globs. Otherwise, is_glob is false
		let is_glob = if expand_glob { check_globs(token.text().into()) } else { expand_glob };
		let is_brace_expansion = helper::is_brace_expansion(token.text());
		let is_cmd_sub = matches!(token.tk_type,TkType::CommandSub);

		if is_cmd_sub {
			let new_token = expand_cmd_sub(token)?;
			product_buffer.push_back(new_token);
			continue
		}

		let expand_home = check_home_expansion(token.text());
		if expand_home {
			// If this unwrap fails, god help you
			let home = read_vars(|vars| vars.get_evar("HOME").unwrap())?;
			token.wd.text = token.wd.text.replace("~",&home);
		}

		if !is_glob && !is_brace_expansion {
			if token.text().has_unescaped("$") && !token.wd.flags.intersects(WdFlags::FROM_VAR | WdFlags::SNG_QUOTED) {
				if token.text().has_unescaped("$@") {
					let mut param_tokens = expand_params(token)?;
					while let Some(param) = param_tokens.pop_back() {
						working_buffer.push_front(param);
					}
					continue
				}
				token.wd.text = expand_var(token.text().into())?;
			}
			if helper::is_brace_expansion(token.text()) || token.text().has_unescaped("$") {
				working_buffer.push_front(token);
			} else {
				if expand_home {
					// If this unwrap fails, god help you
					let home = read_vars(|vars| vars.get_evar("HOME").unwrap())?;
					token.wd.text = token.wd.text.replace("~",&home);
				}
				product_buffer.push_back(token)
			}

		} else if is_brace_expansion && token.text().has_unescaped("{") && token.tk_type != TkType::String {
			// Perform brace expansion
			let expanded = expand_braces(token.text().to_string(), VecDeque::new());
			for mut expanded_token in expanded {
				expanded_token = expand_var(expanded_token)?;
				working_buffer.push_back(
					Tk {
						tk_type: TkType::Expanded,
						wd: WordDesc {
							text: expanded_token,
							span: token.span(),
							flags: token.flags()
						}
					}
				);
			};
		} else if is_glob {
			// Expand glob patterns
			for path in glob(token.text()).unwrap().flatten() {
				working_buffer.push_back(
					Tk {
						tk_type: TkType::Expanded,
						wd: WordDesc {
							text: path.to_str().unwrap().to_string(),
							span: token.span(),
							flags: token.flags()
						}
					}
				);
			}
		} else if let Some(alias_content) = read_logic(|log| log.get_alias(token.text()))? {
			let alias_content = alias_content.split(' ');
			for word in alias_content {
				working_buffer.push_back(
					Tk {
						tk_type: TkType::Expanded,
						wd: WordDesc {
							text: word.into(),
							span: token.span(),
							flags: token.flags()
						}
					}
				);
			}
		} else {
			if expand_home {
				// If this unwrap fails, god help you
				let home = read_vars(|vars| vars.get_evar("HOME").unwrap())?;
				token.wd.text = token.wd.text.replace("~",&home);
			}
			product_buffer.push_back(token);
		}
	}

	product_buffer.map_rotate(|mut elem| {
		elem.wd.text = elem.wd.text.consume_escapes();
		elem
	});
	if split_words {
		split_tokens(&mut product_buffer);
	}
	Ok(product_buffer)
}

pub fn split_tokens(tk_buffer: &mut VecDeque<Tk>) {
    let mut new_buffer = VecDeque::new();

    while let Some(tk) = tk_buffer.pop_front() {
			let split = tk.text().split_outside_quotes();
			for word in split {
				new_buffer.push_back(Tk {
					tk_type: TkType::String,
					wd: WordDesc {
						text: word,
						span: tk.span(),
						flags: tk.flags(),
					}
				});
			}
    }

    // Replace the original buffer with the new one
    *tk_buffer = new_buffer;
}

pub fn clean_escape_chars(token_buffer: &mut VecDeque<Tk>) {
	for token in token_buffer {
		let mut text = std::mem::take(&mut token.wd.text);
		text = text.replace('\\',"");
		token.wd.text = text;
	}
}

pub fn expand_cmd_sub(token: Tk) -> RshResult<Tk> {
	let new_token;
	if let TkType::CommandSub = token.tk_type {
		let body = token.text().to_string();
		let node = Node {
			command: None,
			nd_type: NdType::Subshell { body, argv: VecDeque::new() },
			flags: NdFlags::VALID_OPERAND | NdFlags::IN_CMD_SUB,
			redirs: VecDeque::new(),
			span: token.span()
		};
		let (mut r_pipe,w_pipe) = RustFd::pipe()?;
		let io = ProcIO::from(None,Some(w_pipe.mk_shared()),None);
		execute::handle_subshell(node, io)?;
		let buffer = r_pipe.read()?;
		new_token = Tk {
			tk_type: TkType::String,
			wd: WordDesc { text: buffer.trim().to_string(), span: token.span(), flags: token.flags() }
		};
		r_pipe.close()?;
	} else {
		return Err(ShError::from_internal("Called expand_cmd_sub() on a non-commandsub token"))
	}
	Ok(new_token)
}

pub fn expand_braces(word: String, mut results: VecDeque<String>) -> VecDeque<String> {
	if let Some((preamble, rest)) = word.split_once("{") {
		if let Some((amble, postamble)) = rest.split_last("}") {
			// the current logic makes adjacent expansions look like this: `left}{right`
			// let's take advantage of that, shall we
			if let Some((left,right)) = amble.split_once("}{") {
				// Reconstruct the left side into a new brace expansion: left -> {left}
				let left = format!("{}{}{}","{",left,"}");
				// Same with the right side: right -> {right}
				// This also has the side effect of rebuilding any subsequent adjacent expansions
				// e.g. "right1}{right2}{right3" -> {right1}{right2}{right3}
				let right = format!("{}{}{}","{",right,"}");
				// Recurse
				let left_expanded = expand_braces(left.to_string(), VecDeque::new());
				let right_expanded = expand_braces(right.to_string(), VecDeque::new());
				// Combine them
				for left_part in left_expanded {
					for right_part in &right_expanded {
						results.push_back(format!("{}{}",left_part,right_part));
					}
				}
			} else {
				let mut expanded = expand_amble(amble);
				while let Some(string) = expanded.pop_front() {
					let expanded_word = format!("{}{}{}", preamble, string, postamble);
					results = expand_braces(expanded_word, results); // Recurse for nested braces
				}
			}
		} else {
			// Malformed brace: No closing `}` found
			results.push_back(word);
	}
} else {
	// Base case: No more braces to expand
	results.push_back(word);
}
results
}

pub fn expand_amble(amble: String) -> VecDeque<String> {
	let mut result = VecDeque::new();
	if amble.contains("..") && amble.len() >= 4 {
		let num_range = amble.chars().next().is_some_and(|ch| ch.is_ascii_digit())
				&& amble.chars().last().is_some_and(|ch| ch.is_ascii_digit());

		let lower_alpha_range = amble.chars().next().is_some_and(|ch| ch.is_ascii_lowercase())
				&& amble.chars().last().is_some_and(|ch| ch.is_ascii_lowercase())
				&& amble.chars().next() <= amble.chars().last(); // Ensure valid range

		let upper_alpha_range = amble.chars().next().is_some_and(|ch| ch.is_ascii_uppercase())
				&& amble.chars().last().is_some_and(|ch| ch.is_ascii_uppercase())
				&& amble.chars().next() <= amble.chars().last(); // Ensure valid range

		if lower_alpha_range || upper_alpha_range {
			let left = amble.chars().next().unwrap();
			let right = amble.chars().last().unwrap();
			for i in left..=right {
				result.push_back(i.to_string());
			}
		}
		if num_range {
			let (left,right) = amble.split_once("..").unwrap();
			for i in left.parse::<i32>().unwrap()..=right.parse::<i32>().unwrap() {
				result.push_back(i.to_string());
			}
		}
	} else {
		let mut cur_string = String::new();
		let mut chars = amble.chars();
		let mut brace_stack = vec![];
		while let Some(ch) = chars.next() {
			match ch {
				'{' => {
					cur_string.push(ch);
					brace_stack.push(ch);
				}
				'}' => {
					cur_string.push(ch);
					brace_stack.pop();
				}
				',' => {
					if brace_stack.is_empty() {
						result.push_back(cur_string);
						cur_string = String::new();
					} else {
						cur_string.push(ch)
					}
				}
				'\\' => {
					let next = chars.next();
					if !matches!(next, Some('}') | Some('{')) {
						cur_string.push(ch)
					}
					if let Some(next) = next {
						cur_string.push(next)
					}
				}
				_ => cur_string.push(ch)
			}
		}
		result.push_back(cur_string);
	}
	result
}

pub fn expand_var(mut string: String) -> RshResult<String> {
	let index_regex = Regex::new(r"(\w+)\[(\d+)\]").unwrap();
	loop {
		let mut left = String::new();
		let mut right = String::new();
		let mut chars = string.chars().collect::<VecDeque<char>>();

		while let Some(ch) = chars.pop_front() {
			match ch {
				'\\' => {
					left.push(ch);
					if let Some(next_ch) = chars.pop_front() {
						left.push(next_ch);
					} else {
						break;
					}
				}
				'$' => {
					right.extend(chars.drain(..));
					break;
				}
				_ => left.push(ch),
			}
		}

		if right.is_empty() {
			return Ok(string); // No more variables to expand
		}

		let mut right_chars = right.chars().collect::<VecDeque<char>>();
		let mut var_name = String::new();
		while let Some(ch) = right_chars.pop_front() {
			match ch {
				_ if ch.is_alphanumeric() => {
					var_name.push(ch);
				}
				'_' | '[' | ']' => var_name.push(ch),
				'-' | '*' | '?' | '$' | '@' | '#' => {
					var_name.push(ch);
					break;
				}
				'{' => continue,
				'}' => break,
				_ => {
					right_chars.push_front(ch);
					break;
				}
			}
		}
		let right = right_chars.iter().collect::<String>();

		let value = if index_regex.is_match(&var_name) {
			if let Some(caps) = index_regex.captures(&var_name) {
				let var_name = caps.get(1).map_or("", |m| m.as_str());
				let index = caps.get(2).map_or("", |m| m.as_str());

				read_vars(|v| v.index_arr(var_name, index.parse::<usize>().unwrap()).unwrap())?
			} else {
				return Err(ShError::from_syntax("This is a weird way to index a variable", Span::new()));
			}
		} else {
			read_vars(|vars| vars.get_var(&var_name).unwrap_or_default())?
		};

		let expanded = format!("{}{}{}", left, value, right);

		if expanded.has_unescaped("$") {
			string = expanded; // Update string and continue the loop for further expansion
		} else {
			return Ok(expanded); // All variables expanded
		}
	}
}

fn expand_params(token: Tk) -> RshResult<VecDeque<Tk>> {
	let mut expanded_tokens = VecDeque::new();
	// Get the positional parameter string from shellenv and split it
	let arg_string = read_vars(|vars| vars.get_param("@").unwrap_or_default())?;
	let arg_split = arg_string.split(' ');

	// Split the token's text at the first instance of '$@' and make two new tokens
	// Subsequent instances will be handled in later iterations of expand()
	let (left,right) = token.text().split_once("$@").unwrap();
	let left_token = Tk::new(left.to_string(), token.span(), token.flags());
	let right_token = Tk::new(right.to_string(), token.span(), token.flags());

	// Push the left token into the deque
	if !left_token.text().is_empty() {
		expanded_tokens.push_back(left_token);
	}
	for arg in arg_split {
		// For each arg, make a new token and push it into the deque
		let new_token = Tk::new(arg.to_string(),token.span(), token.flags() | WdFlags::FROM_VAR);
		expanded_tokens.push_back(new_token);
	}
	// Now push the right token into the deque
	if !right_token.text().is_empty() {
		expanded_tokens.push_back(right_token);
	}
	Ok(expanded_tokens)
}
