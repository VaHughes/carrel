#compdef carrel
# zsh completion for carrel. Hand-written; see carrel.bash for why.
_arguments \
  '(- *)--help[show usage]' \
  '(- *)--version[show version]' \
  '--plain[render the document as plain text]:file:_files -g "*.md"' \
  '--render[styled ANSI text: attributes and links, never colours]' \
  '--tasks[print the task list as checkbox lines and exit]' \
  '--diff[read the input as a unified diff]' \
  '--no-diff[never adapt a diff, even on a pipe]' \
  '*:file:_files -g "*.md"'
