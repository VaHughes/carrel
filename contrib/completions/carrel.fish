# fish completion for carrel. Hand-written; see carrel.bash for why.
complete -c carrel -l help    -d 'show usage'
complete -c carrel -l version -d 'show version'
complete -c carrel -l plain   -d 'render the document as plain text'
complete -c carrel -k -a '(__fish_complete_suffix .md)'
