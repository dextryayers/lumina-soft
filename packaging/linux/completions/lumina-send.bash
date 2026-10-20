_lumina_send() {
    local cur prev
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    if [[ $COMP_CWORD -eq 1 ]]; then
        COMPREPLY=( $(compgen -W "status show hide collapse notify set_rgb cycle_rgb set_profile get_config set_config reload_config media player_list shortcut execute spectrum geometry debug_overlay quit --socket --pipe --payload --compact --pretty --quiet --help --completions" -- "$cur") )
    elif [[ "$prev" == "--completions" ]]; then
        COMPREPLY=( $(compgen -W "bash zsh fish" -- "$cur") )
    fi
}
complete -F _lumina_send lumina-send
