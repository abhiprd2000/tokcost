# demo/demo.tape — regenerate the README GIF with VHS:
#   cargo build --release
#   vhs demo/demo.tape
# (run from the repo root; VHS: https://github.com/charmbracelet/vhs)

Output demo/demo.gif

Set Shell "bash"
Set FontSize 18
Set Width 1200
Set Height 680
Set Theme "Dracula"
Set Padding 20
Set TypingSpeed 45ms

Type "export PATH=$PWD/target/release:$PATH && clear"
Enter
Sleep 800ms

Type "tokcost demo/prompt.txt"
Enter
Sleep 2.5s

Type "tokcost --model claude-sonnet-5 demo/prompt.txt"
Enter
Sleep 3s

Type "tokcost --json demo/prompt.txt"
Enter
Sleep 3s

Type "clear"
Enter
Sleep 500ms

Type "tokcost meter --model gpt-4o -- ./demo/fake_stream.sh"
Enter
Sleep 6s

Sleep 1.5s
