;;; texrs-mode.el --- Major mode for TeX, targeting the texrs engine -*- lexical-binding: t; -*-

;; Copyright (c) 2026 MenkeTechnologies

;; Author: MenkeTechnologies
;; URL: https://github.com/MenkeTechnologies/texrs
;; Version: 0.1.0
;; Package-Requires: ((emacs "27.1"))
;; Keywords: languages, tex

;; This file is not part of GNU Emacs.

;;; Commentary:

;; A major mode for TeX documents, targeting the `texrs' engine — Knuth's
;; mouth and expander compiled to fusevm bytecode.  Provides:
;;
;;   - highlighting by CATEGORY CODE, the way the engine reads a file:
;;     comments, control words and control symbols, the primitives texrs
;;     resolves, group braces, math shift, alignment tabs, parameters,
;;     super/subscripts and active characters
;;   - indentation by group depth, counting `{'/`}', \begingroup/\endgroup
;;     and the conditionals
;;   - running the buffer through `texrs' (C-c C-c), with the two
;;     introspection modes on their own keys: the token stream (C-c C-t)
;;     and the lowered bytecode (C-c C-d)
;;   - language-server integration via `texrs --lsp' (eglot + lsp-mode)
;;   - eldoc and completion-at-point over the primitives the engine
;;     dispatches, from `texrs-stdlib' — generated from the engine's own
;;     corpus, so the list cannot offer something that would fail
;;
;; It does NOT claim `.tex' by default.  Emacs ships `tex-mode', and many
;; people run AUCTeX; silently taking every TeX file from either would be
;; a rude thing for a package to do.  Turn it on deliberately:
;;
;;   (add-to-list 'auto-mode-alist '("\\.tex\\'" . texrs-mode))
;;
;; or call `texrs-mode' in a buffer, or put `%% -*- mode: texrs -*-' at the
;; top of a document.
;;
;; The other thing worth knowing: a TeX document decides what its own
;; characters mean.  `\catcode`\@=11' makes `@' a letter, and after it
;; `\intern@l' is one control word.  No editor can know a file's category
;; codes without running it, so the highlighting below assumes plain TeX's
;; table — the one a `.tex' file almost always has.  A document that
;; reassigns a code is coloured by the old meaning; nothing here loses its
;; place because of it.

;;; Code:

(require 'texrs-stdlib)

(declare-function lsp-activate-on "lsp-mode")
(declare-function lsp-register-client "lsp-mode")
(declare-function lsp-stdio-connection "lsp-mode")
(declare-function make-lsp-client "lsp-mode")
;; Declared, not required: both packages are optional, and the blocks that
;; touch these run only after the package that owns them has loaded.
(defvar eglot-server-programs)
(defvar lsp-language-id-configuration)

(defgroup texrs nil
  "Major mode for TeX, targeting the texrs engine."
  :group 'languages
  :prefix "texrs-")

(defcustom texrs-executable "texrs"
  "Path to the texrs executable, used to run buffers and for the language server."
  :type 'string
  :group 'texrs)

(defcustom texrs-indent-offset 2
  "Number of spaces per group level in `texrs-mode'."
  :type 'integer
  :group 'texrs)

(defface texrs-primitive-face
  '((t :inherit font-lock-keyword-face))
  "Face for control sequences texrs itself implements."
  :group 'texrs)

(defface texrs-control-sequence-face
  '((t :inherit font-lock-function-name-face))
  "Face for control sequences a document defines."
  :group 'texrs)

(defface texrs-catcode-face
  '((t :inherit font-lock-builtin-face))
  "Face for the characters whose category code gives them a meaning: $ & # ^ _ ~."
  :group 'texrs)

;;; The primitives, split so the conditionals and group markers can be
;;; recognised by the indenter as well as by font-lock.

(defconst texrs--conditionals
  '("if" "ifcase" "ifcat" "ifcsname" "ifdefined" "ifdim" "ifeof" "iffalse"
    "ifhbox" "ifhmode" "ifinner" "ifmmode" "ifnum" "ifodd" "iftrue" "ifvbox"
    "ifvmode" "ifvoid" "ifx")
  "Control sequences that open a conditional.")

(defconst texrs--block-openers
  (cons "begingroup" texrs--conditionals)
  "Control sequences that deepen a group.")

(defconst texrs--block-closers
  '("fi" "endgroup")
  "Control sequences that close a group.")

(defconst texrs--block-middles
  '("else" "or")
  "Control sequences that sit between the arms of a conditional.")

(defconst texrs--primitives
  '("advance" "catcode" "count" "csname" "def" "divide" "edef" "end"
    "endcsname" "expandafter" "gdef" "global" "ignorespaces" "let" "message"
    "multiply" "noexpand" "number" "par" "relax" "string" "the" "xdef")
  "Control sequences texrs resolves that neither open nor close a group.")

;;; Font-lock.  The order matters: a primitive is a control word, so the
;;; specific rules come before the catch-all.

(defconst texrs-font-lock-keywords
  `((,(concat "\\\\" (regexp-opt (append texrs--block-openers
                                         texrs--block-closers
                                         texrs--block-middles)
                                 t)
              "\\_>")
     . 'texrs-primitive-face)
    (,(concat "\\\\" (regexp-opt texrs--primitives t) "\\_>")
     . 'texrs-primitive-face)
    ;; A control word the document defined, or one the engine does not have.
    ("\\\\[A-Za-z]+" . 'texrs-control-sequence-face)
    ;; A control symbol is a backslash and exactly one character, whatever
    ;; that character would mean on its own: `\%' is not a comment.
    ("\\\\[^A-Za-z]" . 'texrs-control-sequence-face)
    ;; A macro parameter.
    ("#[0-9]?" . font-lock-variable-name-face)
    ;; The characters that mean something because of their category code.
    ("[$&^_~]" . 'texrs-catcode-face))
  "Font-lock keywords for `texrs-mode'.
Comments and group braces come from the syntax table.")

(defvar texrs-mode-syntax-table
  (let ((st (make-syntax-table)))
    ;; Catcode 14: a comment to end of line.  TeX also swallows the line
    ;; ending, which is why a commented line joins the next one; Emacs
    ;; cannot express that, and colouring it as a comment is what matters.
    (modify-syntax-entry ?% "<" st)
    (modify-syntax-entry ?\n ">" st)
    ;; Catcode 0: the escape character.  Not a string escape — a control
    ;; symbol is a token, and marking it "\\" is what keeps `\%' from
    ;; opening a comment.
    (modify-syntax-entry ?\\ "\\" st)
    ;; Catcodes 1 and 2.
    (modify-syntax-entry ?{ "(}" st)
    (modify-syntax-entry ?} "){" st)
    ;; Ordinary characters that are punctuation to Emacs' parser: none of
    ;; them delimit anything in TeX.
    (modify-syntax-entry ?\" "." st)
    (modify-syntax-entry ?\' "." st)
    (modify-syntax-entry ?$ "." st)
    st)
  "Syntax table for `texrs-mode'.")

;;; Indentation — by group depth, which is what nesting means in TeX.

(defconst texrs--control-word-re "\\\\\\([A-Za-z]+\\)"
  "A control word: the escape character then LETTERS ONLY.
A control word ends at the first non-letter, which is why membership is
tested against the captured name rather than by putting a symbol boundary
in the regexp — `\\ifnum1<2\' is `\\ifnum\' and then `1\', but `ifnum1\'
is one symbol to Emacs.")

(defun texrs--line-starts-with-closer-p ()
  "Non-nil if the line at point begins by closing a group or opening another arm."
  (save-excursion
    (beginning-of-line)
    (skip-chars-forward " \t")
    (or (eq (char-after) ?})
        (and (looking-at texrs--control-word-re)
             (let ((name (match-string 1)))
               (or (member name texrs--block-closers)
                   (member name texrs--block-middles)))))))

(defun texrs--line-delta ()
  "Net group depth the line at point adds: braces plus block control sequences."
  (let ((delta 0)
        (end (line-end-position)))
    (save-excursion
      (beginning-of-line)
      (while (< (point) end)
        (cond
         ;; A comment ends the line for this purpose.
         ((eq (char-after) ?%) (goto-char end))
         ;; A control word: whether it opens or closes is a question about
         ;; its name.
         ((looking-at texrs--control-word-re)
          (let ((name (match-string 1)))
            (cond ((member name texrs--block-openers) (setq delta (1+ delta)))
                  ((member name texrs--block-closers) (setq delta (1- delta)))))
          (goto-char (match-end 0)))
         ;; A control symbol is the escape and exactly one character, so a
         ;; `\{' is a token rather than a group opener.
         ((and (eq (char-after) ?\\) (< (1+ (point)) (point-max)))
          (forward-char 2))
         ((eq (char-after) ?{) (setq delta (1+ delta)) (forward-char 1))
         ((eq (char-after) ?}) (setq delta (1- delta)) (forward-char 1))
         (t (forward-char 1)))))
    delta))

(defun texrs-indent-line ()
  "Indent the current line by the group depth it sits in."
  (interactive)
  (let ((indent 0))
    (save-excursion
      (beginning-of-line)
      (let ((this-closes (texrs--line-starts-with-closer-p)))
        (when (zerop (forward-line -1))
          (while (and (looking-at-p "[ \t]*$") (zerop (forward-line -1))))
          (setq indent (max 0 (+ (current-indentation)
                                 (* texrs-indent-offset (texrs--line-delta))))))
        ;; A line that starts by closing has already been counted by the
        ;; line above it; put it back one level.
        (when this-closes
          (setq indent (max 0 (- indent texrs-indent-offset))))))
    (if (<= (current-column) (current-indentation))
        (indent-line-to indent)
      (save-excursion (indent-line-to indent)))))

;;; Running the buffer.

(defun texrs--run (&rest flags)
  "Run the buffer's file through texrs with FLAGS, in a compilation buffer."
  (unless buffer-file-name
    (user-error "Buffer is not visiting a file; save it first"))
  (when (and (buffer-modified-p) (y-or-n-p "Save buffer before running? "))
    (save-buffer))
  (require 'compile)
  (compile (mapconcat #'shell-quote-argument
                      (append (list texrs-executable) flags (list buffer-file-name))
                      " ")))

(defun texrs-run-buffer ()
  "Run the current buffer's document through texrs.
The document is passed positionally: texrs takes `texrs FILE.tex'."
  (interactive)
  (texrs--run))

(defun texrs-dump-tokens ()
  "Show the mouth's token stream for this buffer, without expanding anything."
  (interactive)
  (texrs--run "--dump-tokens"))

(defun texrs-disassemble ()
  "Show the fusevm bytecode this buffer lowers to."
  (interactive)
  (texrs--run "--disasm"))

(defvar texrs-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "C-c C-c") #'texrs-run-buffer)
    (define-key map (kbd "C-c C-t") #'texrs-dump-tokens)
    (define-key map (kbd "C-c C-d") #'texrs-disassemble)
    map)
  "Keymap for `texrs-mode'.")

;;; eldoc and completion, over the primitives the engine dispatches.

(defun texrs--control-sequence-at-point ()
  "The control sequence at point, backslash included, or nil."
  (save-excursion
    (let ((end (point)))
      (skip-chars-backward "A-Za-z")
      (when (eq (char-before) ?\\)
        (buffer-substring-no-properties (1- (point)) end)))))

(defun texrs-eldoc-function (&rest _)
  "Return the eldoc line for the primitive at point, or nil."
  (texrs-stdlib-signature (texrs--control-sequence-at-point)))

(defun texrs-completion-at-point ()
  "`completion-at-point-functions' entry: complete primitive names."
  (save-excursion
    (let ((end (point)))
      (skip-chars-backward "A-Za-z")
      (when (eq (char-before) ?\\)
        (list (1- (point)) end texrs-primitive-names :exclusive 'no)))))

;;; LSP.  texrs speaks it over stdio with `--lsp' and nothing else; an
;;; appended `--stdio' is not a flag it has.

(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs
               `(texrs-mode . (,texrs-executable "--lsp"))))

(with-eval-after-load 'lsp-mode
  (add-to-list 'lsp-language-id-configuration '(texrs-mode . "tex"))
  (when (fboundp 'lsp-register-client)
    (lsp-register-client
     (make-lsp-client
      :new-connection (lsp-stdio-connection
                       (lambda () (list texrs-executable "--lsp")))
      :activation-fn (lsp-activate-on "tex")
      :server-id 'texrs-lsp))))

;;;###autoload
(define-derived-mode texrs-mode prog-mode "texrs"
  "Major mode for TeX documents, targeting the texrs engine.

\\{texrs-mode-map}"
  :syntax-table texrs-mode-syntax-table
  (setq-local font-lock-defaults '(texrs-font-lock-keywords))
  (setq-local comment-start "% ")
  (setq-local comment-start-skip "%+[ \t]*")
  (setq-local comment-end "")
  (setq-local indent-line-function #'texrs-indent-line)
  (setq-local indent-tabs-mode nil)
  (setq-local tab-width texrs-indent-offset)
  (add-hook 'completion-at-point-functions #'texrs-completion-at-point nil t)
  (if (boundp 'eldoc-documentation-functions)
      (add-hook 'eldoc-documentation-functions #'texrs-eldoc-function nil t)
    (setq-local eldoc-documentation-function #'texrs-eldoc-function)))

;;;###autoload
(add-to-list 'interpreter-mode-alist '("texrs" . texrs-mode))

(provide 'texrs-mode)
;;; texrs-mode.el ends here
