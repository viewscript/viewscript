# vsc add-component

Add a visual component (RoundedRect, Circle, etc.)

## Usage

```
vsc add-component [OPTIONS] --component-type <COMPONENT_TYPE>
```

## Options

Options:
  -t, --component-type <COMPONENT_TYPE>
          Component type (RoundedRect, Circle, Line, Path)
  -x, --x <X>
          X position [default: 0]
  -y, --y <Y>
          Y position [default: 0]
  -w, --width <WIDTH>
          Width (for RoundedRect) [default: 100]
  -h, --height <HEIGHT>
          Height (for RoundedRect) [default: 100]
  -r, --radius <RADIUS>
          Corner radius (for RoundedRect) [default: 0]
  -f, --fill <FILL>
          Fill: solid color "#ff6b6b" or gradient "linear-gradient(to right, #ff0000, #00ff00)" [default: #888888]
  -s, --stroke <STROKE>
          Stroke: "width:color" format (e.g., "2:#000000")
  -h, --help
          Print help

## Related

- [Index](../index.md)
