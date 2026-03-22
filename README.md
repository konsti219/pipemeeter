# pipemeeter

A PipeWire frontend inspired by VoiceMeeter.
The goal is to provide a simple UI to easily route audio between any inputs and outputs
while dynamically adapting to a changing PipeWire graph and then keeping that graph in a desired shape.

# Configuring

The configuration is done through the GUI. When the app is first started it will be mostly empty and you will have no audio. But it should only take a minute to get the basics setup.

## Strips

The UI consists of four main sections, each of which is made up of multiple "strips". Eachs strip represents a logical audio path. The strips are:

- **Physical In**: Represents a physical audio input device, such as a microphone.
- **Physical Out**: Represents a physical audio output device, such as speakers or headphones.
- **Virtual In**: Represents a collection of virtual audio producers, typically any application that produces audio, such as a music player or a web browser.
- **Virtual Out**: Represents a collection of virtual audio consumers, typically any application that uses microphone input, for example Discord.

An arbitrary number strips can be created in each category.

PipeWire nodes are be "matched" to exactly one strip using various criteria, including name, description, media name and process name. By default a strip in a category will only match nodes which belong to the same category (e.g. Physical In only matches physical microphones). However, this can be disabled for each strip individually.

The physical strips only ever match a single node, while the virtual strips can match multiple nodes. This allows multiple programs to be routed with one strip and since all programs need to be routed somewhere the virtual strips also come with a default strip which will pickup all programs which are not explicitly matched by any other strip.

## Routing

Routing is done by selecting to which Out strips each In strip is connected. This allows for flexible routing like playing audio ove multiple devices at once or routing your music to your virtual Discord microphone.

## Volume Control

Each strip also has a volume control which allows you to adjust the volume of all nodes routed through that strip at once.

# Runtime Behavior

On startup the current PipeWire graph is read and mostly nuked, only other meter nodes are left alone.
Pipemeeter does **not** restore anything on exit, only tears down its own nodes. So you will likely be left with an empty graph and no audio. Either backup whatever state you had yourself or re-trigger your existing setup by interacting with your systems default audio management. The systemt default audio routing is not disabled by Pipemeeter, but its graph output is simply overridden.

After startup the graph is built up again according to the current configuration, creating new nodes and links as necessary. After that the graph is monitored for changes and adapted to them, for example when a new program playing, which will be matched into an existing strip and routed accordingly.

# Running / Development

I recommend using [nix](https://nixos.org/download.html) to run and develop pipemeeter, but you can also build it with cargo if you have the right dependencies (Wayland/X11 stuff) installed.

```bash
nix run .
```

You can also add this repo as a flake input to properly install the app.

```nix
{
  environment.systemPackages = [
    inputs.pipemeeter.packages.${pkgs.stdenv.hostPlatform.system}.default
  ];
}
```