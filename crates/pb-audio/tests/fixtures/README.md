Test audio: a 1 s 440 Hz tone (stereo, 44.1 kHz, half scale), made with ffmpeg:

    ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=1" -ac 2 -af "volume=0.5" tone.wav
    ffmpeg -i tone.wav -c:a libmp3lame -b:a 96k tone.mp3     # and likewise libvorbis (.ogg), libopus (.opus, .webm),
                                                              # aac (.m4a), flac (.flac)
