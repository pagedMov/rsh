# Ox Shell
#### 🚧 Currently Under Construction 🚧
This project is currently in extremely early production. Documentation is currently limited, and it is most likely going to experience many growing pains as it is developed. Expect many breaking changes if you wish to use it.

Ox is a modern, customizable shell program written in Rust that aims to push the capabilities of shell scripting while maintaining familiarity with traditional shells like bash and zsh.

---

## 🚀 Current Features

### Dynamic Prompts
- Supports all of the basic escape sequences from Bash, plus:
	- **Custom Prompt Scripting**: Dynamically display context-specific information in your prompt using custom escape sequences. Define sequences with `setopt` and access them in your prompt using `\{` and `\}`. For example:
		```bash
		setopt prompt.custom.gitbranch="git branch --show-current 2> /dev/null"
		export PS1="\(on \{gitbranch\}\n\)"
		```
		This would dynamically display the current Git branch in your prompt.
	- **Exit Status Indicators**: Show symbols for success (`\S`) or failure (`\F`), or expand the exit code directly (`\\?`).
	- **Vis Groups**: The `\(` and `\)` sequences dynamically show or hide prompt content based on the current context. If none of the inner escape sequences expand into anything, everything in the group is hidden. For example:
		```bash
		\(on \{gitbranch\} \([\{gitsigns\}]\)\n\)
		```
		This expands to `on dev[!?]` when inside a Git repository (with `gitsigns` and `gitbranch` custom sequences defined) and is hidden when outside a Git repo.


### Interpreter-Agnostic Subshells
- **Shebang support:**
Subshells can be given a shebang (`#!`) to allow for the contained text to be executed using any interpreter. Subshell shebangs can be given just the command name, and the path will be expanded if a corresponding command is found in your PATH. For instance:
```
(#!python
print("hello world")
) | (#!bash
read -r line
echo $line | sed 's/hello/goodbye/'
)
```
Output: `goodbye world`
- **Subshell Arguments:**
Subshells can be given arguments the same way that regular scripts can. Example:
```
(#!python
import sys
def multiply(left, right):
    print(int(left) * int(right))

multiply(sys.argv[0], sys.argv[1])
) 2 4
```
Output: `8`

### Variable Typing
Ox has builtins which allow for the definition of strongly typed variables. This feature is still in very early development, so type-specific interactions have not been implemented just yet.
- **Features:**
	- **Standard Variable Assignment:**
	Generic, bash-like assignment like `i="foo"` still works, and produces weakly typed variables.
	- **Typed Assignment:**
	Variables can be assigned a type using builtins. The syntax is similar to using `export`. Example: `float i=1.5`
- **Currently Implemented Types:**
	- **float:** Ox comes with out of the box support for floating point arithmetic, without a dependency on external tools such as `bc`.
	- **int:** Signed 32-bit integers.
	- **array:** Type-agnostic arrays, similar to Python's lists. Example: `arr list=[1, "foo", 3.5]`
		When accessed, arrays are printed as `1 foo 3.5` in the case of the previous example.
		Note that array manipulation has not yet been implemented, so they are currently immutable structures.

### Detailed Error Output
Ox has a detailed error output mechanism that will show you the exact line and area in that line where a script or command failed, similar to interpreters in modern scripting languages such as Python. For example, the command `if true; then echo foo; fi; done` will produce this error:
```
foo
1;26 - Found `done` outside of loop context

if true;then echo foo;fi;done
                         ^~~^
```

## 🚧 Feature Roadmap
This project aims to improve the general experience of using the shell, in the contexts of both scripting and general interactive use. As such, many of the features I hope to implement are going to be quality of life features, or improvements that break limitations commonly associated with shell scripting.
- **Macros:**
	- Something similar to Rust's macro system, which would allow for dynamic expansion of syntax at script run-time. Functionally, this would exist as something in-between aliases and functions, and would allow for interesting functionality.
- **Builtins for Common Tasks:**
	- It is my opinion that shells rely too heavily on external tools for extremely basic tasks. Using something like awk for simple field extraction, or sed for simple string replacement, feels like extreme overkill given how powerful those commands actually are. I would like to implement builtins that provide these simple functionalities in a way that doesn't force users to remember the various flags and bespoke syntax rules of tools like awk and sed.
- **Language Server:**
	- This is a distant feature, but implementing a language server that would not only provide diagnostics for Ox code, but also diagnostics for code in subshells that use different interpreters, would be extremely nice.

## Contributing
I welcome contribution of any kind on this project. If you'd like to contribute, feel free to fork the repo and submit a pull request.

## Why Ox?
Shell scripting is one of my favorite things to do on a computer, but there’s no denying its limitations. Shells like Bash often struggle with tasks that are trivial in other programming languages. Advanced math, detailed string manipulation—these operations are nearly impossible without relying on external tools. And even with access to these external dependencies, you need to perform a ridiculous level of syntax gymnastics to achieve the desired result, as each of these tools has different rules you have to follow.

With Ox, my goal is to break these limitations and provide an ergonomic, modern environment for writing shell scripts. There’s simply no reason why shell scripting can’t be as frictionless and expressive as writing a Python script.
