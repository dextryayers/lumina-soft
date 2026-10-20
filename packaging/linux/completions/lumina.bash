_lumina() {
    local cur prev
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"

    case "$prev" in
        --config|--init-config|--output|--baseline|--check-theme)
            COMPREPLY=( $(compgen -f -- "$cur") )
            return 0
            ;;
        --export-theme)
            COMPREPLY=( $(compgen -W "default midnight neon sunset mono glass nord catppuccin high-contrast" -- "$cur") )
            return 0
            ;;
        --autostart)
            COMPREPLY=( $(compgen -W "status install remove" -- "$cur") )
            return 0
            ;;
    esac

    if [[ $COMP_CWORD -eq 1 ]]; then
        COMPREPLY=( $(compgen -W "--version -V --help -h --diagnostics -D --check-config -C --config --init-config --force --print-config --list-themes --export-theme --check-theme --self-test --benchmark --output --baseline --autostart --activate --hidden" -- "$cur") )
    fi
}

complete -F _lumina lumina
