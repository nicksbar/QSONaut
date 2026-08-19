ISSUE
CW decoder sucks. We need to take another run at implementation with ditdah. USB-D seems like the mode we'd wanna use for this for digital interfacing, not CW mode directly for my 7300.

DONE
VBW setting now defaults off, explains its smoothing behavior on hover, and persists with the operator profile.

DONE
Decode decks now use bounded in-flow containers instead of nested top panels, so expanding the contact log cannot place them over other panels.

ISSUE
RX monitoring still doesn't produce any audio.

TODO
Lessons learned from ft8 need to flow into ft4.

TODO
We need to build out all the other digital modes supported in the app, with the lessons learned.
Focus on HF ones first, then WSPR. Disable the UHF modes, i don't have a radio. Group them at the end of the panel and disable the access.

ISSUE
The automation gets stuck. If a cycle is incomplete, partially complete, or previously started - we are stuck. I can break that cycle by responding manually or restarting the app.

FEATURE REQUEST
We should support common radio interfaces, like rigctrl, dxlab, other...

FEATURE REQUEST
Standard UDP logging client only. 

TASK
Readme update, lots changed. Still alpha, but lets focus on capabilities.


