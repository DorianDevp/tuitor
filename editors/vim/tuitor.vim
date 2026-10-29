" tuitor from Vim and Neovim: F9 toggles a breakpoint on the cursor's line, F8 shows that
" line in tuitor. Source this file from your vimrc or init.vim, or copy it into ~/.vim/plugin/.
function! s:Tuitor(command) abort
  let l:place = expand('%:p') . ':' . line('.')
  echo trim(system('tuitor send ' . a:command . ' ' . shellescape(l:place)))
endfunction

command! TuitorBreakpoint call s:Tuitor('bp toggle')
command! TuitorShow call s:Tuitor('show')
nnoremap <silent> <F9> :TuitorBreakpoint<CR>
nnoremap <silent> <F8> :TuitorShow<CR>
