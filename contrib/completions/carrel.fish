# fish completion for carrel. Hand-written; see carrel.bash for why.
complete -c carrel -l help    -d 'show usage'
complete -c carrel -l version -d 'show version'
complete -c carrel -l plain   -d 'render the document as plain text'
complete -c carrel -l diff    -d 'read the input as a unified diff'
complete -c carrel -l no-diff -d 'never adapt a diff, even on a pipe'
complete -c carrel -k -a '(__fish_complete_suffix .md)'
