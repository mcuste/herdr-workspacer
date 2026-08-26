Output docs/demo/herdr-workspacer-demo.gif

Set Shell zsh
Set Theme "Catppuccin Mocha"
Set FontSize 17
Set Width 1600
Set Height 850
Set Padding 20
Set TypingSpeed 60ms

Hide
Type "./docs/demo/start-herdr-workspacer-demo.sh"
Enter
Sleep 12s
Set TypingSpeed 1ms
Type "set -gx CLOUDSDK_CONFIG /tmp/hw/gcloud; set -gx CLOUDSDK_CORE_ACCOUNT ''; set -gx CLOUDSDK_CORE_PROJECT ''; set -e CLOUDSDK_ACTIVE_CONFIG_NAME; clear"
Enter
Sleep 1s
Set TypingSpeed 60ms
Show
Sleep 2s

# Open workspaces first, then the remaining zoxide directories.
Hide
Set TypingSpeed 1ms
Ctrl+L
Type "herdr plugin action invoke open --plugin herdr-workspacer >/dev/null 2>&1; clear"
Enter
Sleep 2s
Set TypingSpeed 60ms
Show
Sleep 1s

# Filter to a directory that is not open, then create its workspace.
Type "api"
Sleep 2s
Hide
Enter
Sleep 4s
Set TypingSpeed 1ms
Type "set -gx CLOUDSDK_CONFIG /tmp/hw/gcloud; set -gx CLOUDSDK_CORE_ACCOUNT ''; set -gx CLOUDSDK_CORE_PROJECT ''; set -e CLOUDSDK_ACTIVE_CONFIG_NAME; clear"
Enter
Sleep 1s
Set TypingSpeed 60ms
Show

# The new workspace is now the most recent entry.
Hide
Set TypingSpeed 1ms
Ctrl+L
Type "herdr plugin action invoke open --plugin herdr-workspacer >/dev/null 2>&1; clear"
Enter
Sleep 2s
Set TypingSpeed 60ms
Show
Sleep 1s
Type "dia"
Sleep 2s
Hide
Enter
Sleep 4s
Set TypingSpeed 1ms
Type "set -gx CLOUDSDK_CONFIG /tmp/hw/gcloud; set -gx CLOUDSDK_CORE_ACCOUNT ''; set -gx CLOUDSDK_CORE_PROJECT ''; set -e CLOUDSDK_ACTIVE_CONFIG_NAME; clear"
Enter
Sleep 1s
Set TypingSpeed 60ms
Show

Hide
Set TypingSpeed 1ms
Ctrl+L
Type "herdr plugin action invoke open --plugin herdr-workspacer >/dev/null 2>&1; clear"
Enter
Sleep 2s
Set TypingSpeed 60ms
Show
Down
Sleep 700ms
Down
Sleep 700ms
Down
Sleep 1s
Type "infra"
Sleep 2s
Ctrl+U
Sleep 2s
Escape
Sleep 2s
