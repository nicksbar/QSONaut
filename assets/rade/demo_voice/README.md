# Synthetic RADE demo voice

These short WAV files are generated locally with `espeak-ng` for QSONaut's
null-modem RADE demonstration. They are intentionally synthetic and use
obviously fictional callsigns. They are not recordings of W1AW or any other
real station.

Format: 16 kHz, mono, signed 16-bit PCM.

The source phrases and generation settings are:

```text
espeak-ng -v en-us+f3 -s 145 -p 45 "CQ CQ, this is K7ZZZ calling. Is anyone up for a friendly chat?"
espeak-ng -v en-us+f2 -s 145 -p 45 "K7ZZZ, this is W9LOL. Your signal is five nine, and your antenna is taller than my house."
espeak-ng -v en-us+f3 -s 145 -p 45 "Did you build that antenna, or did it come with the radio?"
espeak-ng -v en-us+f2 -s 145 -p 45 "I built it from wire, two zip ties, and unreasonable optimism."
espeak-ng -v en-us+f3 -s 145 -p 45 "What is your grid square?"
espeak-ng -v en-us+f2 -s 145 -p 45 "My grid is November Zero Zero. The weather is clear and the coffee is strong."
espeak-ng -v en-us+f3 -s 145 -p 45 "Please repeat. My cat is sitting on the radio desk."
espeak-ng -v en-us+f2 -s 145 -p 45 "Thanks for the contact. Seventy three from W9LOL."
```

eSpeak NG is used only to create these checked-in demo fixtures; QSONaut does
not require an installed TTS engine at runtime.
