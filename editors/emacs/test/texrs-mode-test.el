;;; texrs-mode-test.el --- Tests for texrs-mode -*- lexical-binding: t; -*-

;;; Commentary:

;; Run with:
;;
;;   emacs -Q --batch -L editors/emacs -l editors/emacs/test/texrs-mode-test.el \
;;         -f ert-run-tests-batch-and-exit

;;; Code:

(require 'ert)
(require 'texrs-mode)

(defmacro texrs-test--with-buffer (text &rest body)
  "Run BODY in a `texrs-mode' buffer holding TEXT, point at the start."
  (declare (indent 1))
  `(with-temp-buffer
     (insert ,text)
     (goto-char (point-min))
     (texrs-mode)
     (font-lock-ensure)
     ,@body))

(defun texrs-test--face-at (needle)
  "The face on the first character of NEEDLE in the current buffer."
  (goto-char (point-min))
  (search-forward needle)
  (get-text-property (- (point) (length needle)) 'face))

(ert-deftest texrs-test-comment-runs-to-end-of-line ()
  "A per-cent sign comments to end of line; a backslashed one does not."
  (texrs-test--with-buffer "% a comment\n\\message{kept}\n"
    (should (eq (texrs-test--face-at "a comment") 'font-lock-comment-face))
    (should-not (eq (texrs-test--face-at "kept") 'font-lock-comment-face)))
  ;; `\%' is a control symbol: the rest of the line is not a comment.
  (texrs-test--with-buffer "\\% still text\n"
    (should-not (eq (texrs-test--face-at "still") 'font-lock-comment-face))))

(ert-deftest texrs-test-primitives-are-told-from-user-macros ()
  "A control sequence the engine has is faced differently from one it does not."
  (texrs-test--with-buffer "\\def\\greet#1{\\message{#1}}\n"
    (should (eq (texrs-test--face-at "\\def") 'texrs-primitive-face))
    (should (eq (texrs-test--face-at "\\greet") 'texrs-control-sequence-face))
    (should (eq (texrs-test--face-at "\\message") 'texrs-primitive-face))))

(ert-deftest texrs-test-comment-syntax-is-set-for-the-comment-commands ()
  "`comment-region' and friends need the comment strings TeX uses."
  (texrs-test--with-buffer "\\relax\n"
    (should (equal comment-start "% "))
    (should (equal comment-end ""))))

(ert-deftest texrs-test-indentation-follows-group-depth ()
  "A group deepens the next line; the line that closes it comes back out."
  (texrs-test--with-buffer "\\begingroup\n\\def\\a{A}\n\\endgroup\n"
    (indent-region (point-min) (point-max))
    (let ((lines (split-string (buffer-string) "\n")))
      (should (equal (nth 0 lines) "\\begingroup"))
      (should (equal (nth 1 lines) "  \\def\\a{A}"))
      (should (equal (nth 2 lines) "\\endgroup"))))
  ;; A conditional is a group too, and \else is one of its arms rather than
  ;; another level.
  (texrs-test--with-buffer "\\ifnum1<2\n\\message{yes}\n\\else\n\\message{no}\n\\fi\n"
    (indent-region (point-min) (point-max))
    (let ((lines (split-string (buffer-string) "\n")))
      (should (equal (nth 1 lines) "  \\message{yes}"))
      (should (equal (nth 2 lines) "\\else"))
      (should (equal (nth 4 lines) "\\fi")))))

(ert-deftest texrs-test-a-brace-in-a-control-symbol-is-not-a-group ()
  "`\\{' is a token, so it must not open a level."
  (texrs-test--with-buffer "\\catcode`\\{=1\n\\relax\n"
    (indent-region (point-min) (point-max))
    (should (equal (nth 1 (split-string (buffer-string) "\n")) "\\relax"))))

(ert-deftest texrs-test-completion-offers-primitives-after-a-backslash ()
  "Completion at point covers the control sequence being typed."
  (texrs-test--with-buffer "\\mess"
    (goto-char (point-max))
    (let ((capf (texrs-completion-at-point)))
      (should capf)
      ;; The range starts at the backslash: replacing only the letters would
      ;; leave `\\message' spelled `\\\\message'.
      (should (eq (char-after (nth 0 capf)) ?\\))
      (should (= (nth 1 capf) (point-max)))
      (should (member "\\message" (nth 2 capf))))))

(ert-deftest texrs-test-eldoc-describes-a-primitive-and-nothing-else ()
  "Eldoc answers for a primitive, and stays quiet for a macro it cannot know."
  (texrs-test--with-buffer "\\message"
    (goto-char (point-max))
    (let ((line (texrs-eldoc-function)))
      (should (stringp line))
      (should (string-match-p "message" line))))
  (texrs-test--with-buffer "\\greet"
    (goto-char (point-max))
    (should-not (texrs-eldoc-function))))

(ert-deftest texrs-test-the-primitive-table-comes-from-the-engine ()
  "The generated table is present and holds what the engine dispatches."
  (should (> (length texrs-primitive-names) 20))
  (should (member "\\csname" texrs-primitive-names))
  (should (member "\\ifnum" texrs-primitive-names))
  ;; Every name in the table has a line for eldoc to show.
  (dolist (name texrs-primitive-names)
    (should (stringp (texrs-stdlib-signature name)))))

(ert-deftest texrs-test-tex-files-are-not-claimed-without-being-asked ()
  "The mode does not take `.tex' from tex-mode or AUCTeX by loading."
  (should-not (rassq 'texrs-mode auto-mode-alist)))

(provide 'texrs-mode-test)
;;; texrs-mode-test.el ends here
