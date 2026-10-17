# sr5500p

A command-line tool for the KING JIM TEPRA PRO SR5500P (USB `0d8a:0103`) on Linux.

```
cargo run -- status                         # printer state and loaded tape
cargo run -- print test-pattern
cargo run -- print text 'Hello'
cargo run -- print text A-01 A-02 A-03          # three labels in one job
cargo run -- print qr 'https://example.com/'
cargo run -- print image logo.png
cargo run -- print --dry-run --tape 12 text 'Hello'   # preview.png only
```

The user needs read/write access to `/dev/usb/lpN` (print data) and
`/dev/bus/usb/BBB/DDD` (status). On Debian/Ubuntu both belong to the `lp`
group: `sudo usermod -aG lp $USER`.

## Protocol notes

- Print data is written to the usblp device as-is. No session handshake is needed.
- Status: vendor control request `bmRequestType=0xC1, bRequest=0x01, wLength=8`
  returns `[08, state, error, tape width code, ...]`. It works while usblp is bound.
  State goes 0 (idle) → 2 (printing) → 5 (print end) → 0.
- 180 dpi. Printable dots across the tape: 4mm 22, 6mm 36, 9mm 54, 12mm 72, 18mm 108, 24mm 128.
- Cut (`C`): `02 02 01 01` cuts after each label. The SDK value `01 01 01 01`
  makes SR5500P cut about 10 mm before the end of the label.
- Every job starts by cutting off ~12 mm of blank tape (the distance between
  the head and the cutter). Labels within one job are cut without waste, so
  multiple labels are sent as a single job.
- Job: `@ { C D G`, then per label `L T` + one `ESC .` raster line per column + `0C`, then `@`.

## References

- [hikalium/sr5900p](https://github.com/hikalium/sr5900p) — SR5900P over the network; same command set at 360 dpi
- [nyacom/tepraprint](https://github.com/nyacom/tepraprint) — SR920 over USB; status request
- [nikachu2012/tepra_web PROTOCOL.md](https://github.com/nikachu2012/tepra_web/blob/main/PROTOCOL.md) — TEPRA SDK analysis (model table, commands, printable sizes)
