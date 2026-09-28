[Interaction Spec Changes]
- [x] Change pane switching from <leader> hjkl to <alt> hjkl
- [x] Change pane moving from <leader> HJKL to <ctrl> <alt> hjkl
- [x] Change carry from <leader> c to <ctrl> <alt> 1,2,...

[Additional Additions]
- [x] Add a visible frame around the panels using smooth glyphs
- [x] Add a popup menu when the leader key is pressed showing the keybinds
- [x] Tabs should autocreate if navigated to and deleted if empty when navigated away from
- [x] Top bar should show a tab number followed by the number of panes in it. ie |1 - 4|2 - 1/2| if tab 2 pane 1 is focused

Implemented and covered by fullscreen PTY tests. Newly visited tabs start a shell;
empty tabs are removed when left. Ctrl-Alt-digits use CSI-u or modifyOtherKeys
encoding; see README.md for legacy-terminal mappings.
