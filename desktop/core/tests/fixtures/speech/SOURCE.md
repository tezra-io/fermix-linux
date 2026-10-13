`jfk_48k_s16le.raw` is the first 1.504 s (94 frames of 16 ms) of `samples/jfk.wav` from
https://github.com/ggerganov/whisper.cpp at commit `b0a11594aec50892a02cd8d129eee2dfe93a8bb8`
(sha256 `59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e`). It holds about
320 ms of room tone, then "And so my fellow Americans".

The recording is John F. Kennedy's inaugural address of 20 January 1961, a work of the United
States federal government and so in the public domain.

Made with ffmpeg 4.4.2, raw PCM16 LE mono at 48 kHz, the rate the capture branch hands to
`speech::Uplink`:

```
ffmpeg -i jfk.wav -ac 1 -ar 48000 -f s16le -t 1.504 jfk_48k_s16le.raw
```

Re-make it with that command; never hand-edit it.
