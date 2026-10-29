;;; tuitor.el --- Breakpoints in tuitor from Emacs  -*- lexical-binding: t; -*-

;; F9 toggles a breakpoint on the current line, F8 shows that line in tuitor.
;; Load it from your init file: (load "/path/to/tuitor.el")

(require 'subr-x)

(defun tuitor--send (command)
  "Run COMMAND in tuitor with the current file and line."
  (let ((place (format "%s:%d" (buffer-file-name) (line-number-at-pos))))
    (message "%s" (string-trim
                   (shell-command-to-string
                    (concat "tuitor send " command " " (shell-quote-argument place)))))))

(defun tuitor-breakpoint ()
  "Toggle a tuitor breakpoint on the current line."
  (interactive)
  (tuitor--send "bp toggle"))

(defun tuitor-show ()
  "Show the current line in tuitor."
  (interactive)
  (tuitor--send "show"))

(global-set-key (kbd "<f9>") #'tuitor-breakpoint)
(global-set-key (kbd "<f8>") #'tuitor-show)

(provide 'tuitor)
;;; tuitor.el ends here
