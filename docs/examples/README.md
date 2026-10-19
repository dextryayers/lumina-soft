# IPC examples

## `make-notify.sh`

Runs `make` with the arguments it receives and reports the result through
`lumina-send`. The exit status is preserved, so it works as a drop-in shell
alias or as the command in a Makefile target.

```sh
chmod +x docs/examples/make-notify.sh
docs/examples/make-notify.sh test
```

To use it for every interactive `make`, put a symlink named `make` earlier in
`PATH` than `/usr/bin/make`.

## `github-actions-reporter.yml`

Copy it to `.github/workflows/` and adjust the workflow name and runner labels.
The job must run on a self-hosted runner inside the desktop session that owns
the Lumina socket. A GitHub-hosted runner has no access to that socket.

## `mpv-osd.lua`

Copy it into mpv's script directory, for example
`~/.config/mpv/scripts/lumina-osd.lua`, or load it with
`mpv --script=docs/examples/mpv-osd.lua`. It reports file changes and
pause/resume to the island.

Configuration goes in `~/.config/mpv/script-opts/lumina.conf`:

```ini
lumina_send=lumina-send
notify_pause=yes
```
