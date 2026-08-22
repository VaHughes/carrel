#compdef carrel
# zsh completion for carrel. Hand-written; see carrel.bash for why.
_arguments \
  '(- *)--help[show usage]' \
  '(- *)--version[show version]' \
  '--plain[render the document as plain text]:file:_files -g "*.md"' \
  '--diff[read the input as a unified diff]' \
  '--no-diff[never adapt a diff, even on a pipe]' \
  '*:file:_files -g "*.md"'
