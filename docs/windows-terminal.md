# Windows Terminal shortcuts

Multiplexer keeps the same bindings on Windows and Unix. Windows Terminal
handles its shortcuts before the application receives input, so native Windows
support alone cannot release a shortcut captured by the host.

## Required for the default carry shortcuts

Windows Terminal binds Ctrl+Alt+1–8 to its first eight tabs and Ctrl+Alt+9 to
its last tab. These conflict with Multiplexer's carry-to-tab commands, including
the leader-prefixed variants. Ctrl-b, Alt-h/j/k/l, Ctrl-Alt-h/j/k/l, Alt-[ / Alt-],
and Alt-1–9 have no matching host default.
[Windows Terminal defaults](https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalSettingsModel/defaults.json).

Open Windows Terminal Settings → Open JSON file. Merge the nine entries from
[windows-terminal.settings.json](windows-terminal.settings.json) into the
existing top-level `keybindings` array. If a key already has a custom entry,
replace that entry with the provided `unbound` entry. Preserve the rest of your
settings. An `unbound` entry forwards the key to the application.
[Microsoft keybinding documentation](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/actions#unbind-keys-disable-keybindings).

These overrides apply to **every Windows Terminal profile**. They release the
host's numbered tab shortcuts; Ctrl+Tab and Ctrl+Shift+Tab still navigate host
tabs. Multiplexer does not edit Windows Terminal settings automatically.

## Optional passthrough for programs inside a pane

Keep Ctrl+Shift+C and Ctrl+Shift+V for host copy/paste. Multiplexer copies its
own selection with leader `y`, then `v`/`V` and `y`. Windows Terminal also
ships Ctrl+V as paste; release it if a hosted editor needs Ctrl+V, such as Vim's
block selection. Ctrl+Shift+P opens the host command palette; release it if a
hosted editor needs that key. Append only the entries you need:
[host defaults](https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalSettingsModel/defaults.json),
[initial user defaults](https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalSettingsModel/userDefaults.json).

```json
{ "keybindings": [
  { "id": "unbound", "keys": "ctrl+v" },
  { "id": "unbound", "keys": "ctrl+shift+p" }
] }
```

Other host shortcuts, such as Alt+arrows, Ctrl+Shift+F, and F11, likewise need
individual overrides when a hosted program needs them. They do not overlap
Multiplexer's built-in commands. Ctrl+C copies a host selection when one exists;
clear that selection to send Ctrl+C to the pane.
[Windows Terminal actions](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/actions),
[selection handling](https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalControl/ControlInteractivity.cpp).

## Mouse, paste, and keyboard layouts

Windows Terminal reserves Shift+mouse for its own selection/scrolling. Plain
wheel events reach Multiplexer, but its Shift-wheel history override cannot
operate when the host consumes the event. Use leader `y` and the selection-mode
history keys to browse pane history. Shift-drag and Ctrl+Shift+C copy the visible
host display, which can include borders and other panes.
[mouse handling](https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalControl/ControlInteractivity.cpp).

Multiplexer requests bracketed paste so pasted text bypasses its command router.
Use the host paste action rather than a `sendInput` action that synthesizes key
sequences. Keep `experimental.input.forceVT` at its default `false` for native
Windows console input. If exact pasted whitespace matters, set `trimPaste` to
`false`; Windows Terminal otherwise trims trailing whitespace before sending it.
[interaction settings](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/interaction).

Windows Terminal's profile setting `altGrAliasing` defaults to `true`, allowing
Ctrl+Alt to behave like AltGr. For a dedicated Multiplexer profile, consider
`"altGrAliasing": false` if Ctrl+Alt combinations produce layout characters
instead of shortcuts. Multiplexer preserves Ctrl+Alt symbols and Unicode text;
AltGr-generated ASCII letters/digits remain ambiguous with its direct shortcuts.
Rebind those commands in Multiplexer's TOML configuration when necessary.
[AltGr profile setting](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/profile-advanced#altgr-aliasing).
