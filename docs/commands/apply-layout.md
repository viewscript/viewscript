# vsc apply-layout

Apply a layout combinator to arrange instances (Phase 13)

## Usage

```
vsc apply-layout [OPTIONS] --instances <INSTANCES> <LAYOUT_TYPE>
```

## Options

Options:
      --instances <INSTANCES>  Instance IDs as JSON array, e.g., "[101, 102, 103]"
      --anchor <ANCHOR>        Anchor point (TL, TR, BL, BR) [default: TL]
      --gap <GAP>              Gap between instances (rational, e.g., "16" or "32/2") [default: 0]
      --origin-x <ORIGIN_X>    X origin for first instance
      --origin-y <ORIGIN_Y>    Y origin for first instance
      --intent <INTENT>        Natural language intent
  -h, --help                   Print help

## Related

- [Index](../index.md)
