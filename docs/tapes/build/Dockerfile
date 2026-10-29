# The build that the `build` demo debugs: its last step fails, and tuitor says why.
FROM alpine:3.20
RUN packages="jq curl"
RUN apk add --no-cache $packages && jq --version
