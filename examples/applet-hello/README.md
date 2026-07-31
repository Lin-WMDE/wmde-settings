# Giving your applet a settings page

WMDE Settings builds an applet's settings page out of a file, not out of code. Ship one
file with your package and your applet gets a page - correctly styled, translated,
searchable - without Settings being rebuilt, patched, or even aware that your applet
exists.

This directory is a working example. It contains no applet at all: just the two files an
applet would ship, so you can install it and see the result.

```
fun.example.AppletHello.desktop   what makes it an applet
fun.example.AppletHello.ron       what makes it configurable
PKGBUILD                          how the two get installed
```

## Try it

```sh
makepkg -si
```

Then restart Settings, open **Panel -> Applets**, and press the gear on any row - or on
`Hello (settings example)` in the **Add applet** drawer. Nothing was recompiled.

To take it away again:

```sh
sudo pacman -R wmde-applet-hello
```

## The two files

### 1. The desktop entry

You already ship this; an applet without it appears nowhere. The only key that matters
here is `X-CosmicApplet=true`, which is what makes the panel and Settings treat the entry
as an applet rather than an application.

```ini
[Desktop Entry]
Name=Hello (settings example)
Comment=Demonstrates the WMDE applet settings schema
Type=Application
Exec=/usr/bin/true
Icon=face-smile-symbolic
NoDisplay=true
X-CosmicApplet=true
```

Settings takes the page's **title, description and icon** from `Name`, `Comment` and
`Icon`, including their `[uk]`-style translations. Your schema never repeats them, so the
two can never disagree.

> The example's `Exec` runs `/usr/bin/true`, because there is no applet binary here. Yours
> points at your applet.

### 2. The schema

```
/usr/share/wmde/applet-settings/<desktop entry id>.ron
```

The file name **is** the id of your `.desktop` file - here
`fun.example.AppletHello.ron` for `fun.example.AppletHello.desktop`. That is the whole
registration mechanism: there is no key to add, no daemon to notify, no list to join.

Settings searches `XDG_DATA_HOME` first and then every directory in `XDG_DATA_DIRS`, so
while you are writing one you can shadow the packaged file from
`~/.local/share/wmde/applet-settings/` without touching the system.

## Writing the schema

```
(
    schema: 1,
    config: (id: "fun.example.AppletHello", version: 1),
    groups: [
        (
            title: { "": "Group heading", "uk": "Заголовок групи" },
            rows: [ ... ],
        ),
    ],
)
```

- `schema` is the version of the schema **language**, not of your applet. Use `1`.
- `config` says which config the page writes: an id and a version, exactly as you pass
  them to `Config::new`. It does not have to be your own - the notifications applet's page
  edits the notification daemon's config - but say so on purpose.
- Each group becomes one card. Each row becomes one line in it.

Every label is a small map of language to text. The `""` key is the mandatory English
fallback; a language that is missing falls back to it.

```
label: { "": "Loudness", "uk": "Гучність", "de": "Lautstärke" }
```

There is no compiler behind these strings. A typo in a language code degrades quietly to
English rather than failing to build - a real cost of the file being data.

### Controls

| `control` | For a field of type | Notes |
|---|---|---|
| `Toggle` | `bool` | |
| `Choice(items: [...])` | anything | up to 4 options render as radio buttons, 5+ as a dropdown; force it with `style: Radio` or `style: Dropdown` |
| `Number(min:, max:)` | any integer or float | `step` defaults to 1, `decimals` to 0; add `suffix` for a unit; `optional: true` for `Option<T>` |
| `Slider(min:, max:)` | any number | add `min_label` and `max_label` |
| `Text` | `String` | `placeholder`, `max_len` |

Plus three rows that hold no value:

| Row | Use it for |
|---|---|
| `Note(text:)` | a setting that exists but is not editable here - a list, a map, something the applet maintains itself |
| `Link(page:, label:)` | a setting that already has its own page in Settings; `page` is that page's id, such as `sound` or `power` |
| `External(exec:, label:)` | anything a file cannot describe: a network search, a file picker, several keys written from one answer |

### Values are raw text

A `default`, and every `Choice` value, is written into the config **exactly as you spell
it**. That is not a shortcut - it is what lets the file describe types it has never heard
of:

```
default: "Celsius"                     a unit enum variant
default: "Some(5000)"                  an Option<u32>
default: "\"Kyiv\""                    a String, quoted
value: "Some(ActiveWorkspace)"         an Option<enum>
```

Anything your field's `Deserialize` accepts can be written here. Text you type into a
`Text` control is quoted and escaped for you; you only write RON by hand in `default` and
in `Choice` values.

Get it wrong and the row is dropped with a warning in the log rather than writing rubbish
into your config. Run Settings with `RUST_LOG=warn` while you are writing a schema.

### What your applet must do

Nothing special, and probably nothing new:

- store its settings with `cosmic-config` under the id and version the schema declares;
- subscribe with `core.watch_config(APP_ID)`.

Settings writes **one key at a time** and never the whole struct, so keys your schema does
not mention are left alone - including keys another program owns. With the subscription in
place, a change takes effect immediately; without it, at the applet's next start.

Do not put secrets in a schema. Config files are written with the ordinary umask and are
world-readable.

## Packaging

Two `install` lines. No dependency on any WMDE crate, no linking, no build step:

```sh
install -Dm0644 fun.example.AppletHello.desktop \
  "$pkgdir/usr/share/applications/fun.example.AppletHello.desktop"
install -Dm0644 fun.example.AppletHello.ron \
  "$pkgdir/usr/share/wmde/applet-settings/fun.example.AppletHello.ron"
```

File names are namespaced by your applet id, so two packages cannot collide.

## Limits worth knowing before you start

- **The page appears at the next start of Settings.** The applet list is read once.
- **No apply or cancel.** Every change is written immediately, as everywhere else in
  Settings.
- **Reset writes defaults, it does not clear keys.** The button at the foot of the page
  restores the value your package installed under `/usr/share/wmde/<config id>/v<N>/`, or
  the schema's own `default`. A key with neither is left as it is: there is no way to unset
  one.
- **A schema cannot compute.** No conditional rows, no validation beyond the range you
  declare, no value that depends on another. When you need any of that, `External` is the
  honest answer.
- **If you bump your config version, bump it in the schema too.** Nothing checks this, and
  the symptom is a setting that appears to do nothing.
