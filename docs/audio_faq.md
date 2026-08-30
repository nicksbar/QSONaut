# Audio FAQ

## QSONaut says “NO INPUT” on Windows. What does that mean?

QSONaut opened the selected recording device but did not receive samples from
Windows before the input timeout. This is usually a device-selection,
Windows microphone privacy, USB audio routing, or driver problem. It is not
the same as receiving silence: a received signal reports an audio level.

## Can QSONaut detect microphone permission problems?

Windows does not expose a reliable, portable distinction between a denied
desktop-app microphone permission and a capture driver that has stopped
delivering callbacks. QSONaut records callback and stream-error diagnostics so
the two cases can be separated when possible. A zero callback count with no
stream error is consistent with permission, routing, or driver blockage, but
does not prove which one occurred.

## What should I check first?

- In **Settings > Devices**, explicitly select the radio’s USB audio codec;
  do not rely on **System default**.
- In Windows, open **Settings > Privacy & security > Microphone** and enable
  **Microphone access** and **Let desktop apps access your microphone**.
- In **Settings > System > Sound > Input**, select the same USB codec and
  confirm that its input meter moves while the radio is receiving.
- Confirm the FTDX10 USB audio/data settings and reconnect the USB cable.
- Open the codec's Windows sound-device properties, select **Advanced**, and
  consider clearing **Allow applications to take exclusive control of this
  device** and **Give exclusive mode applications priority**. Exclusive-mode
  access can prevent QSONaut from receiving callbacks even when the device
  appears available.
- Close other applications that may be using the codec, then use **Restart
  audio now** in QSONaut.

## What do the diagnostic counters mean?

- **Callback count**: number of capture callbacks delivered by the audio API.
- **Callback frames**: raw device frames delivered to those callbacks.
- **Error callback count**: capture-stream errors reported by the driver/API.
- **Last callback elapsed**: time after stream startup when the last callback
  was observed.

Zero callbacks and zero frames means the stream opened but no capture data
arrived. Nonzero callbacks with a timeout points more toward a conversion,
queue, or stream-liveness issue. A nonzero error count points toward the
Windows audio driver or device.

## Does CAT control affect audio?

No. Yaesu CAT uses the serial connection; USB audio capture is a separate
Windows audio endpoint. CAT timeouts such as `FA;` or `MD;` should be
troubleshot separately from missing audio.