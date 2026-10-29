# tuitor from Kakoune: source this file from your kakrc.
# <user> b toggles a breakpoint on the cursor's line, <user> t shows that line in tuitor.
define-command tuitor-breakpoint -docstring 'toggle a tuitor breakpoint on this line' %{
    info %sh{ tuitor send bp toggle "$kak_buffile:$kak_cursor_line" 2>&1 }
}
define-command tuitor-show -docstring 'show this line in tuitor' %{
    info %sh{ tuitor send show "$kak_buffile:$kak_cursor_line" 2>&1 }
}
map global user b ': tuitor-breakpoint<ret>' -docstring 'tuitor: breakpoint'
map global user t ': tuitor-show<ret>' -docstring 'tuitor: show line'
