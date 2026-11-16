# qsictl

A developer's QSI console (`tools/qsictl`), for Linux and Windows. It
pairs with a QS/VS host, asks it each request of QSI's families and
prints the answer, follows its events, moves a dump or an image, and
attaches to a partition's console in a terminal. It is in no image.

```
cargo build -p qsictl
```

It keeps its identity and the hosts it paired with in its directory:
`$XDG_CONFIG_HOME/qsictl`, `~/.config/qsictl`, or `%APPDATA%\qsictl` on
Windows; `--dir` names another. `--host` (`-H`) names a host by the name
it was paired as; with one host it is that one.

## Pairing

```
qsictl init                         # make this console's identity
qsictl pair 192.0.2.10 ABCDEFGHJKMNPQRS --as vs1
qsictl hosts
qsictl forget vs1
```

The password is the one Service Tools shows (option 4, 2). `pair` prints
the host's certificate fingerprint: it must be the one on that panel.

## Requests

Each prints the reply whole, or every message of the refusal.

| Command | Request |
|---|---|
| `identify`, `state`, `time`, `codes [--limit N]` | base |
| `power [--restart] [--immediate] [--delay N]` | base `Power` |
| `resources`, `settings`, `hardware` | host |
| `list`, `get ID`, `adapters [ID]` | partitions |
| `create NAME`, `delete ID [--disks]`, `activate ID [--profile P] [--source a\|b\|recorded\|d]`, `shutdown ID [--kind delayed\|system\|immediate] [--delay N] [--restart]`, `restart ID` | partition |
| `profiles ID`, `profile ID NAME`, `save-current ID NAME [--revision N]`, `set-default ID NAME` | profiles |
| `history ID [--limit N]` | refcodes |
| `panel ID FUNCTION` | panel |
| `dumps ID`, `take-dump ID` | dumps |
| `screen ID` | console `Screen` |
| `pools`, `vdisks [POOL]`, `images`, `volumes`, `drives ID`, `switches`, `ports [SWITCH]` | vres |

## Streams and events

```
qsictl events [--since N]            # every event, until interrupted
qsictl console ID [--take-over]      # the screen drawn; each line typed is sent
qsictl download ID DUMP FILE
qsictl upload NAME FILE              # waits for the upload's end
```

At a console, a line typed is its characters and Enter; `:F3`, `:enter`,
`:up`, `:pageup`, `:tab`, `:exit`, `:attn`, `:sysreq` and the like are
one key alone. A serial console's lines are sent as its bytes.
