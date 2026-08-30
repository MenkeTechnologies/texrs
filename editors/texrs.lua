-- Neovim configuration for texrs, as a lua module.
-- Usage: require('texrs') from your init.lua, after putting this on the path.
--
-- Same server as editors/texrs.vim sets up, expressed for a lua-first config.

local M = {}

--- Start `texrs --lsp` for TeX buffers.
--- @param opts table|nil { cmd = { 'texrs', '--lsp' }, filetypes = { 'tex', 'plaintex' } }
function M.setup(opts)
  opts = opts or {}
  local cmd = opts.cmd or { 'texrs', '--lsp' }
  local filetypes = opts.filetypes or { 'tex', 'plaintex' }

  vim.api.nvim_create_autocmd('FileType', {
    group = vim.api.nvim_create_augroup('texrs_lsp', { clear = true }),
    pattern = filetypes,
    callback = function(args)
      if vim.fn.executable(cmd[1]) == 0 then
        return
      end
      vim.lsp.start({
        name = 'texrs',
        cmd = cmd,
        root_dir = vim.fs.dirname(vim.fs.find({ '.git' }, { upward = true })[1])
          or vim.fn.getcwd(),
      }, { bufnr = args.buf })
    end,
  })
end

--- Run the current document and return its output.
function M.run()
  return vim.fn.system({ 'texrs', vim.api.nvim_buf_get_name(0) })
end

--- The fusevm bytecode the current document lowers to.
function M.disasm()
  return vim.fn.system({ 'texrs', '--disasm', vim.api.nvim_buf_get_name(0) })
end

return M
