#compdef carrel
# zsh completion for carrel. Hand-written; see carrel.bash for why.
_arguments \
  '(- *)--help[show usage]' \
  '(- *)--version[show version]' \
  '--plain[render the document as plain text]:file:_files -g "*.md"' \
  '*:file:_files -g "*.md"'
