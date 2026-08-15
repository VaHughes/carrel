# bash completion for carrel — a quiet place to read your markdown.
# Hand-written: carrel's CLI is six forms and does not use clap, so adding a
# CLI framework just to generate this would be the tail wagging the dog.
_carrel() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    if [[ "$cur" == -* ]]; then
        COMPREPLY=( $(compgen -W "--help --version --plain" -- "$cur") )
    else
        COMPREPLY=( $(compgen -f -X '!*.md' -- "$cur") $(compgen -d -- "$cur") )
    fi
}
complete -o filenames -F _carrel carrel
