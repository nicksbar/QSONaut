IN PROGRESS
CW decoding is being reworked around USB-D reception. The decoder now searches around the selected tone and estimates a local noise floor; continuous boundary handling and broader noisy-radio validation remain.


IN PROGRESS
RX monitoring now has a live volume control and a 700 Hz test tone for separating output-device problems from radio-input problems. Hardware/system-audio validation is still required.

TODO
Lessons learned from ft8 need to flow into ft4.

IN PROGRESS
Digital modes are now grouped into HF/primary and other/experimental sections. MSK144 is disabled because this station has no UHF radio; FT4 timing hardening and an initial WSPR Type-1 transmitter are now in place, while WSPR scheduling/duty-cycle controls and richer native-mode workflows remain.

FEATURE REQUEST
We should support common radio interfaces, like rigctrl, dxlab, other...

FEATURE REQUEST
Standard UDP logging client only. 

TASK
Readme update, lots changed. Still alpha, but lets focus on capabilities.


