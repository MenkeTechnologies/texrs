" Vim/Neovim configuration for texrs.
" Usage: source this file from your vimrc, or copy the pieces you want.
"
" texrs implements TeX's mouth and expander. It serves plain TeX; a LaTeX
" document will report the first primitive it does not carry rather than
" guessing at one it does not have.

augroup texrs_filetype
  autocmd!
  autocmd BufNewFile,BufRead *.tex setlocal commentstring=%\ %s
augroup END

" Native LSP (Neovim 0.8+): `texrs --lsp` gives completion over the primitives,
" hover from the same reference the docs are generated from, and diagnostics
" from the engine's own lowerer -- a document the editor shows as clean is a
" document texrs will run.
if has('nvim-0.8')
lua <<LUA
local group = vim.api.nvim_create_augroup('texrs_lsp', { clear = true })
vim.api.nvim_create_autocmd('FileType', {
  group = group,
  pattern = { 'tex', 'plaintex' },
  callback = function(args)
    if vim.fn.executable('texrs') == 0 then
      return
    end
    vim.lsp.start({
      name = 'texrs',
      cmd = { 'texrs', '--lsp' },
      root_dir = vim.fs.dirname(vim.fs.find({ '.git' }, { upward = true })[1])
        or vim.fn.getcwd(),
    }, { bufnr = args.buf })
  end,
})
LUA
endif

" :Texrs runs the current document and shows its \message stream.
command! -bar Texrs echo system('texrs ' . shellescape(expand('%:p')))

" :TexrsDisasm opens the fusevm bytecode the document lowered to.
command! -bar TexrsDisasm new | put =system('texrs --disasm ' . shellescape(expand('#:p'))) | setlocal buftype=nofile
