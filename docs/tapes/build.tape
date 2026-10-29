# tuitor build on a Dockerfile whose last step fails, through BuildKit's debugger (docker buildx
# 0.38 or later): the build stops on the failed step, and tuitor says what failed and why.
# From the repository's root: vhs docs/tapes/build.tape (TUITOR=/path/to/tuitor).
Source docs/tapes/settings.tape
Output assets/demo/build.webm
Output assets/demo/build.mp4
Require docker

Hide
Type "source docs/tapes/home.sh && mv ../docs/tapes/build ../shop && cd ../shop && clear" Enter
Sleep 500ms
Show
Sleep 500ms
Type "cat Dockerfile"
Sleep 400ms
Enter
Sleep 2.5s
Type "tuitor build"
Sleep 600ms
Enter
Wait+Screen@240s /stop #1/
Sleep 1.5s
Type "go"
Sleep 2s
Screenshot assets/demo/build.png
Sleep 4s

Hide
Type "Q"
Sleep 2s
