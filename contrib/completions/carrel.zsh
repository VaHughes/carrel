#compdef carrel
# zsh completion for carrel. Hand-written; see carrel.bash for why, and for why
# the glob is wider than `*.md`.
_arguments \
  '(- *)'{-h,--help}'[show usage]' \
  '(- *)'{-V,--version}'[show version]' \
  '--plain[render the document as plain text]:file:_files -g "*.(md|markdown|diff|patch)"' \
  '--render[styled ANSI text: attributes and links, never colours]' \
  '--tasks[print the task list as checkbox lines and exit]' \
  '--diff[read the input as a unified diff]' \
  '--no-diff[never adapt a diff, even on a pipe]' \
  '--no-mouse[hand the pointer back to the terminal]' \
  '*:file:_files -g "*.(md|markdown|diff|patch)"'
