# fish completion for carrel. Hand-written; see carrel.bash for why, and for
# why the suffixes are wider than `.md`.
complete -c carrel -s h -l help    -d 'show usage'
complete -c carrel -s V -l version -d 'show version'
complete -c carrel -l plain   -d 'render the document as plain text'
complete -c carrel -l render  -d 'styled ANSI text: attributes and links, never colours'
complete -c carrel -l tasks  -d 'print the task list as checkbox lines and exit'
complete -c carrel -l diff    -d 'read the input as a unified diff'
complete -c carrel -l no-diff -d 'never adapt a diff, even on a pipe'
complete -c carrel -l no-mouse -d 'hand the pointer back to the terminal'
complete -c carrel -k -a '(__fish_complete_suffix .md)'
complete -c carrel -k -a '(__fish_complete_suffix .markdown)'
complete -c carrel -k -a '(__fish_complete_suffix .diff)'
complete -c carrel -k -a '(__fish_complete_suffix .patch)'
