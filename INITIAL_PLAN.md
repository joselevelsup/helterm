# The VIBE terminal

## Description

I want to build a terminal emulator through the use of GPUI from the creators of zed. But
this is not just a standard terminal emulator. I want this to be a Project Terminal
Emulator. The way I want it to function is that Terminal based editors are FIRST CLASS
CITIZENS.

So when working with a terminal emulator it's literally just the terminal. There
should be workspaces that are dedicated to projects. Each workspace should have tabs and
splits. Make the splits manual though. Similar to the Sway Window manager. Each window woulc
be a container that would be a mini terminal emulator OR editor. For the editor, I want it
to focus on Helix.

A new editor container should be able to spin up the helix terminal command.

From the editor, I should be able to trigger commands that would happen in the workspace like moving to another container or another tab.
In the workspace, there should be a file explorer that can be triggered from the editor. When the file gets clicked on, it should open in the editor or be able to open the editor in another container with the file open.


## Stack
- Rust
- For the terminal rendering library, I want the `alacritty_terminal` crate to be used. 
- GPUI for the UI

## Looks
- The containers should be their own windows and tiled automatically. Similar to the Sway window manager.
- On the left hand side should be the workspace list
- Editor containers should be able to access the file explorer only. The file explorer should always be next to an editor container.
