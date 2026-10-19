`speech-opus.ogg` and `speech-vorbis.ogg` are `core/tests/fixtures/speech/jfk_48k_s16le.raw`
(1.504 s: about 320 ms of room tone, then "And so my fellow Americans"; public domain, see that
folder's `SOURCE.md`) encoded in an Ogg container, once as Opus, the codec a phone's voice note
uses, and once as Vorbis, an Ogg this app does not play. The Vorbis one must fall back to a file
tile.

Made in the GNOME 50 SDK (GStreamer 1.26.11, libopus 1.5.2, libvorbis 1.3.7, libogg 1.3.6) from
`desktop/`:

```
front="filesrc location=core/tests/fixtures/speech/jfk_48k_s16le.raw ! rawaudioparse use-sink-caps=false format=pcm pcm-format=s16le sample-rate=48000 num-channels=1 ! audioconvert"
gst-launch-1.0 -q $front ! opusenc ! oggmux ! filesink location=app/tests/fixtures/voice/speech-opus.ogg
gst-launch-1.0 -q $front ! vorbisenc ! oggmux ! filesink location=app/tests/fixtures/voice/speech-vorbis.ogg
```

Re-make them with those commands; never hand-edit them. oggmux picks a random stream serial, so
a re-made file differs in those bytes; `gst-discoverer-1.0` reads both as 1.504 s, mono, 48 kHz.
