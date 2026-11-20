# Manual Installation

This guide covers manual installation, updates and removal.

## 1. Requirements

- `macOS 11` or later on a Mac with Apple silicon.
- `Rust stable` through [rustup](https://rustup.rs/).
- `Xcode Command Line Tools`, `Git` and `make`.

Install missing command line tools with `xcode-select --install`.

### 1.1. Compatibility

- The build and automated checks passed with `Rust 1.98.1` on `macOS 27.2.1` (`Apple Silicon`).

## 2. Install

Open a new terminal and follow these steps.

```bash
git clone https://github.com/wereallcode/hm.git "$HOME/.hm"
```

```bash
cd "$HOME/.hm" && make build
```

Run `hm` with this command.

```bash
./dist/hm list
```

### 2.1. Add to `PATH`

Follow these steps to add `hm` to your `PATH` and run it from any directory.

```zsh
vi ~/.zshrc
```

Add this line at the end of the file and save it.

```zsh
export PATH="$HOME/.hm/dist:$PATH"
```

Apply the change.

```zsh
source ~/.zshrc
```

Run `hm setup` once to authorize password-free changes to `/etc/hosts`.

```zsh
hm setup
```

Run `hm`. 🥳

```zsh
hm list
```

## 3. Update

Follow these steps to update your local copy and rebuild.

```bash
cd "$HOME/.hm" && git pull --ff-only
```

```bash
make build && hm setup
```

## 4. Uninstall

1. Run `hm setup --remove` to revoke authorization.
2. Remove the `PATH` line from your shell startup file.
3. Open a new terminal.
4. Delete `$HOME/.hm` if you no longer need the source.
