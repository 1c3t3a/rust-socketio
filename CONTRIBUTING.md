# Introduction

Contributions to this project are welcome! 

This project is still being developed, our goal is to have a well-designed
thought-out project, so when you make a contribution, please think in those
terms. That is:

- For code:
    - Is the contribution in the scope of this crate?
    - Is it well documented?
    - Is it well tested?
- For documentation:
    - Is it clear and well-written?
    - Can others understand it easily?
- For bugs:
    - Does it test functionality of this crate?
    - Do you have a minimal crate that causes the issue that we can use and test?
- For feature requests:
    - Is the request within the scope of this crate?
    - Is the request clearly explained?

## Licensing and other property rights.

All contributions that are made to this project are only accepted under the
terms in the [LICENSE](LICENSE) file. That is, if you contribute to this
project, you are certifying that you have all the necessary rights to make the
contribution under the terms of the [LICENSE](LICENSE) file, and you are in fact
licensing them to this project and anyone that uses this project under the terms
in the [LICENSE](LICENSE) file.

## Misc
- We are looking at adopting the [conventional commits 1.0](https://www.conventionalcommits.org/en/v1.0.0/) standard.
    - This would make it easier for us to use tools like [jilu](https://crates.io/crates/jilu) to create change logs.
    - Read [keep a changelog](https://keepachangelog.com/en/1.0.0/) for more information.

## Git hooks

> Git hooks are scripts that Git executes before or after events such as: commit, push, and receive. Git hooks are a built-in feature - no need to download anything. Git hooks are run locally.
- [githooks.com](https://githooks.com/)

These example hooks enforce some of these contributing guidelines so you don't need to remember them.

### pre-commit

Put the contents below in ./.git/hooks/pre-commit
```bash
#!/bin/bash
set -e 
set -o pipefail
# Clippy recomendations
cargo clippy --verbose
# Check formatting
cargo fmt --all -- --check || {
    cargo fmt
    echo "Formatted some files make sure to check them in."
    exit 1
}
# Make sure our build passes
cargo build --verbose
```

### commit-msg


Put the contents below in ./.git/hooks/commit-msg
```bash
#!/bin/bash

# https://dev.to/craicoverflow/enforcing-conventional-commits-using-git-hooks-1o5p

regexp="^(revert: )?(fix|feat|docs|ci|refactor|style|test)(\(.*?\))?(\!)?: .+$"
msg="$(head -1 $1)"

if [[ ! $msg =~ $regexp ]]
then
    echo -e "INVALID COMMIT MESSAGE"
    echo -e "------------------------"
    echo -e "Valid types: fix, feat, docs, ci, style, test, refactor"
    echo -e "Such as: 'feat: add new feature'"
    echo -e "See https://www.conventionalcommits.org/en/v1.0.0/ for details"
    echo
    # exit with an error
    exit 1
fi

while read line
do
    if [[ $(echo "$line" | wc -c) -gt 51 ]]
    then
        echo "Line '$line' is longer than 50 characters."
        echo "Consider splitting into these two lines"
        echo "$line" | head -c 50
        echo
        echo "$line" | tail -c +51
        echo
        exit 1
    fi
done < $1

```
