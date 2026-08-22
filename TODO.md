IN PROGRESS
CW decoding is being reworked around USB-D reception. The decoder now searches around the selected tone and estimates a local noise floor; continuous boundary handling and broader noisy-radio validation remain.


IN PROGRESS
RX monitoring now has a live volume control and a 700 Hz test tone for separating output-device problems from radio-input problems. Hardware/system-audio validation is still required.

DONE
FT4 now shares the hardened native TX path with late-slot rejection and explicit timing telemetry.

IN PROGRESS
Digital modes are now grouped into HF/primary and other/experimental sections. MSK144 is disabled because this station has no UHF radio; FT4 timing hardening and an initial WSPR Type-1 transmitter are now in place, while WSPR scheduling/duty-cycle controls and richer native-mode workflows remain.

FEATURE REQUEST
We should support common radio interfaces, like rigctrl, dxlab, other...

FEATURE REQUEST
Standard UDP logging client only. 

TASK
Readme update, lots changed. Still alpha, but lets focus on capabilities.

CHANGE
The waterfall selector is ok. But isn't clear where i should be selecting. I want to see bandwidth on this, so i know the area/channel we are looking at. I find myself clicking on the center of a signal, when i want to be on the edge. CW should be center yes? And needs to be different then others.
