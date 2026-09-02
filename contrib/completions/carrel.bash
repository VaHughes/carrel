# bash completion for carrel — a quiet place to read your markdown.
# Hand-written: carrel's CLI is six forms and does not use clap, so adding a
# CLI framework just to generate this would be the tail wagging the dog.
#
# The suffixes are the ones carrel actually opens, not just `.md`: the file
# walk accepts `.markdown` too, and `--diff` reads `.diff` and `.patch` as
# documents. A completion narrower than the program is a completion that hides
# files the user can plainly open.
_carrel() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    if [[ "$cur" == -* ]]; then
        COMPREPLY=( $(compgen -W "-h --help -V --version --plain --render --tasks --diff --no-diff --no-mouse" -- "$cur") )
    else
        # extglob is on wherever bash-completion is loaded, which is the only
        # place this file is installed.
        COMPREPLY=( $(compgen -f -X '!*.@(md|markdown|diff|patch)' -- "$cur") $(compgen -d -- "$cur") )
    fi
}
complete -o filenames -F _carrel carrel
