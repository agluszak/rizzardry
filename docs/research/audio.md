# Wizardry 8 audio: retail formats, voice lines, mouth gaps, music

Evidence: reference C++ at `/home/agluszak/code/personal/decomp/whizzardry-8` (paths below are
relative to it), retail data at `/home/agluszak/code/personal/decomp/wizardry-prepared/data`.
Survey script: `specs/audio_survey.py` (`python3 -I audio_survey.py <data dir>`), which reads every
`.wav`, `.mp3` and `.gap` under the tree.

## 1. Formats present (survey of the retail `data/` tree)

No archives or containers. Every sound is a loose file. `levels/` has no audio.

| Where | Count | Format |
|---|---|---|
| `sound/npcs/**`, `sound/pcs/**` (voices) | 11 989 | MP3, MPEG-1 Layer III, 44 100 Hz, **mono, 64 kbps CBR**, no ID3, starting directly with a frame sync (`FF FB 50 CC`) |
| same | 3 | MP3, MPEG-1 L3, 44.1 kHz mono, 192 kbps (`voc_ewaxxcannon2_005a.mp3` and two others) |
| `sound/npcs/**/gap/`, `sound/pcs/**/gap/` | 11 588 | `.gap` mouth-gap tracks (section 4) |
| `sound/{ambients,combat,footsteps,misc,monsters}`, `spells/sounds`, `missiles/sounds` | 4 011 | RIFF WAVE, **format tag 1 (PCM) only**. 16-bit mono 44.1 kHz is most common (3 388 + 198 + 26). Others: 16-bit mono 22 050 (292) and 11 025, 16-bit stereo 44.1k (53) and 22.05k (6), and 8-bit mono 44.1k, 22.05k and 11.025k (~30) |
| `sound/monsters/` | 9 | **headerless `.wav`**: no RIFF header, the bytes look like raw s16le (for example `rapax01fix.wav`, `insect_spice_01.wav`, `ump_vox_atk_thrust_02.wav`). SDL_mixer and hound both reject them. Skip them or wrap them in a header with a guessed rate. |
| `sound/npcs/npc_guard_try_guard/*.wav` (20), `sound/pcs/f_laid20/f_laid20_500.wav` (1) | 21 | voice lines stored as PCM WAV instead of MP3 (same naming) |
| `music/*.mp3` | 23 | MP3, MPEG-1 L3, 44.1 kHz, **joint stereo 160 kbps** (19 of them have ID3v2). `end credit.mp3` is stereo 192 kbps. |
| `music/*.mpl` | 32 | text playlists (section 5) |
| `flics/intro/*.bik` | 7 | Bink video, which carries its own audio. Out of scope. |

No ADPCM (tag 0x11/0x02), no MP3-in-RIFF (0x55) and no Ogg anywhere. The reference engine decodes
everything through SDL_mixer: `MIX_LoadAudio_IO` in `src/audio/soundman.cpp:198`, mixer set to F32
stereo 44 100 in `src/audio/native_audio.cpp:31-33`.

## 2. Path resolution (applies to all sounds)

`resolve_path` (`src/audio/soundman.cpp:90-102`):
1. Upper-case the whole path (`SDL_strupr`). Retail paths are Windows paths using `\`, and lookup is
   case-insensitive. The prepared data is lower-case on disk, so the Bevy port must lower-case or
   case-fold paths.
2. If the file exists, use it.
3. Otherwise swap the extension `.WAV` to `.MP3` (or `.MP3` to `.WAV`).

So scripts ask for `...\NAME_000.wav` and get `name_000.mp3` on disk. This is also why the
try_guard `.wav` voices work.

## 3. NPC voice lines: location, naming and choice

**Stem**: `FormatNpcVoiceSoundPath` (`src/wiz8/local_code/NPC Scripting.cpp:315-321`) builds
`prefix + display name`:
- `RPC_` if the NPC is a party member (`is_grouped`) and this is not a script event;
- else `VOC_` if `record->voice_script != 0` (scripted or cinematic voices);
- else `NPC_`.

The same stem names the dialogue script `Data\NPC Scripts\<stem>.nsf` and the monster script
`<stem>.msf` (`:323-331`).

**File path**: `W8NpcDialoguePanel::speakSubquote` (`src/wiz8/local_code/npc_dialogue_view.cpp:139-185`):
```
Data\Sound\<npcs|PCs>\<stem>\<stem>_<quote:03d>[subquote letter].wav
```
- The directory is `npcs` when the binding is an NPC voice or a forced NPC voice, otherwise `PCs`.
  Recruited RPCs (`rpc_vi`, `rpc_rodan` and so on) are in `sound/pcs/`.
- `quote` is the quote index in the `.nsf`, written as 3 decimal digits.
- Subquote 0 has no suffix. Subquote *n*>0 appends `'a'+n-1` (`npc_anselm_041.mp3`,
  `npc_anselm_041a.mp3`).
- `.wav` is requested, and `resolve_path` falls back to the `.mp3`.
- If `quote->dialogue_alert` is set, the line plays `Data\Sound\npcs\Dialogue Alert.wav` instead
  (`:159`).
- Quote numbers are sparse. Only quotes that have recorded speech have a file. Missing files fall
  back to showing the text (`PlayNpcVoice` returns false, and the bubble is held for
  `graphemes*60+2000` ms, `:224-235`).

**Which quote**: when a conversation opens, `NPCInteractionSubscreen.cpp:3785-3789` queues
**quote 0 (first-meeting greeting) if `greeting_pending`, otherwise quote 1 (repeat hello)**, and
then clears `greeting_pending`. Quote 0 also clears it in `npc_dialogue.cpp:289-290`. Other indices
come from topics and keywords in the `.nsf`. Some fixed ones in the code: 0x1c and 0x1d are
"unknown topic" (picked 50/50), 0x1e is "classified" and 0x12 is price-check (`npc_dialogue.cpp:280-285`,
`NPC Scripting.cpp:575`).

**Layout on disk**: `sound/npcs/` has 144 directories (`npc_*`, `voc_*`). `sound/pcs/` has
personality voices (`f_burly10`, `m_kind20`, ...) and `rpc_*`. Each directory holds
`<stem>_NNN[x].mp3` plus a `gap/` subdirectory.

## 4. Mouth-gap `.gap` format

Loader: `LoadMouthGapTrack` (`src/audio/mouth_gap.cpp:17-43`).
- Path: `<voice dir>\GAP\<voice basename without extension>.gap`, for example
  `sound/npcs/npc_aletheides/gap/npc_aletheides_000.gap`.
- Body: a sequence of **little-endian `int32 start_ms, int32 end_ms` pairs**, with no header and no
  count. The loader reads while 8 or more bytes remain.
- Checked against all 11 588 files: every length is a multiple of 8, and the ranges are sorted and
  do not overlap. The last `end_ms` is less than the clip length. For `npc_aletheides_000`, the last
  range ends at 7 731 ms and the MP3 is 8.528 s long. The first range is almost always `(0, ~35-41)`.
- **Meaning**: each range is a silence ("gap"). `UpdateMouthGapTrack` (`:45-55`) sets the flag named
  `mouth_open` when `start < elapsed_ms < end`. While that flag is set, the 3D head holds mouth
  texture frame 0, the rest pose (`src/entities/monster.cpp:1287-1294`). Outside a gap, the mouth
  picks a random different frame from 0-3 every 120 ms (frames 4 and 5 map to 0, `:1295-1310`).
  Portraits do the same with frames 6-8 (`QuoteManager.cpp:1583-1600`). In short, the **mouth
  flaps everywhere except inside the listed ranges**. The flag name is the reverse of what it does.
- A missing `.gap` file gives an empty track, and the mouth stays closed.

## 5. Music

- Files: `data/music/*.mp3` (MPEG-1 L3 44.1 kHz stereo, 160 kbps).
- Playback: streamed through `wiz8::sound::stream`, with a 5 s fade-in and a 2 s fade-out of the
  previous track when crossfading (`src/wiz8/music_playlist.cpp`, `StartMusicResource`).
- `.mpl` playlists (parser `MusicPlaylist::Parse`, `music_playlist.cpp:95-155`). They are text
  files. Lines starting with `*` are comments, and blank lines are ignored.
  - Track rows are `File Name.mp3(weight)`. Spaces inside the name are kept, and so is the case.
  - `#PAUSEMIN=s`, `#PAUSEMAX=s` and `#PAUSECHANCE=%` set the pauses between songs.
  - Selection is a weighted roll over the summed weights. If any row has no weight, the roll is
    uniform.
  - After each song (outside combat) there is a `PAUSECHANCE`% chance of silence lasting
    `PAUSEMIN + rand(PAUSEMAX-PAUSEMIN)` seconds.
- Example `monastery1.mpl`: `Underground.MP3(70)`, `Outdoor Area.mp3(30)`, `PAUSEMIN=50`,
  `PAUSEMAX=140`, `PAUSECHANCE=80`.

## 6. Proof voice line (Monastery)

`data/sound/npcs/npc_aletheides/npc_aletheides_000.mp3`. This is Aletheides, referenced by
`levels/monastery/monastery2.sts` as `NPC_ALETHEIDES.msf`. Quote 0 is his first-meeting greeting.
- 68 224 bytes. Bare MPEG-1 Layer III frames (first header `FF FB 50 CC`), no ID3 or Xing tag.
  44 100 Hz, mono, 64 kbps CBR, 8.528 s (ffprobe). It decodes cleanly with ffmpeg.
- Companion `gap/npc_aletheides_000.gap`: 80 bytes, 10 ranges:
  `(0,41) (843,881) (2181,2208) (2845,3646) (4109,4347) (5398,5750) (6047,6149) (6356,6422) (7343,7368) (7660,7731)`.
- Alternative: `sound/npcs/npc_anselm/npc_anselm_000.mp3`, same format, 17.66 s, 18 gap ranges.
  Anselm is not in a monastery level file.

**Bevy and rodio**: Bevy's default `mp3` feature (symphonia's MP3 decoder in Bevy 0.12+, minimp3 in
older versions) plays all voice and music files **with no conversion**. Load them as
`AudioSource` from `.mp3`. All WAV SFX are plain PCM 8/16-bit, which hound and symphonia's WAV
support decode directly with the `wav` feature. No ADPCM exists, so no conversion step is needed.
The only exceptions are the 9 headerless `sound/monsters/*.wav` files, which need a RIFF header
added or need skipping. The Vorbis feature is not needed. Bevy's asset loader picks the decoder by
extension, so apply the `.wav`/`.mp3` fallback from section 2 before loading.
