# tuitor from Sublime Text: copy this file into Packages/User/ (Preferences > Browse Packages),
# and the bindings in Default.sublime-keymap into your key bindings.
import os
import subprocess

import sublime
import sublime_plugin


class TuitorCommand(sublime_plugin.TextCommand):
    """Runs `tuitor send <command> file:line` for the cursor's line."""

    def run(self, edit, command="bp toggle"):
        path = self.view.file_name()
        if not path:
            sublime.status_message("tuitor: the file is not saved")
            return
        line = self.view.rowcol(self.view.sel()[0].begin())[0] + 1
        # The project's folder, which tells tuitor the project.
        folders = [f for f in (self.view.window().folders() if self.view.window() else []) if path.startswith(f)]
        cwd = folders[0] if folders else os.path.dirname(path)
        try:
            done = subprocess.run(
                ["tuitor", "send", *command.split(), "{}:{}".format(path, line)],
                cwd=cwd, capture_output=True, text=True, timeout=15,
            )
        except (OSError, subprocess.TimeoutExpired) as err:
            sublime.status_message("tuitor: {}".format(err))
            return
        sublime.status_message("tuitor: " + (done.stdout or done.stderr).strip())
