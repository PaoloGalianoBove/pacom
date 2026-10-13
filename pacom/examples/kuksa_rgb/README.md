# KUKSA RGB actuator example

This one-shot example sends a `#RRGGBB` string to the KUKSA V2 actuator
`Vehicle.Cabin.Light.AmbientLight.Row1.DriverSide.Color`. The CAN provider
continues to handle the existing VSS-to-DBC `Light_RGB` mapping; this example
does not replace the provider, CAN replay, or ambient-temperature application.

The executable accepts a color and an optional VSS signal path. It defaults to
`http://127.0.0.1:55555`, overridable with `PACOM_KUKSA_URI`:

```bash
cargo run --release --example kuksa_rgb -- '#12A4FF'
cargo run --release --example kuksa_rgb -- '#12A4FF' Vehicle.Cabin.Light.AmbientLight.Row1.DriverSide.Color
```

To use it as an Ankaios workload, build and transfer the updated PACOM image,
then apply `docker/ankaios.kuksa-rgb.yaml` while the Databroker and CAN provider
are running. Change the `commandArgs` color in that separate manifest to send a
different RGB value. The workload runs once and exits after the actuation
request is accepted by KUKSA.