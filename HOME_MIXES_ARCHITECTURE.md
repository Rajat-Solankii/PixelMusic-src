# Home Mixes Integration Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              HOME PAGE LAYOUT                                │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐ │
│  │ Quick Picks Shelf                                                     │ │
│  │ [Grid of quick access tiles]                                         │ │
│  └───────────────────────────────────────────────────────────────────────┘ │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐ │
│  │ ★ HOME MIXES (HomeMixes.qml) ★                                       │ │
│  │                                                                        │ │
│  │  ┌──────────────────┐  ┌─────────────────────────────────────────┐   │ │
│  │  │  YOUR MIX       │  │  DAILY MIX                              │   │ │
│  │  │                 │  │ ┌─────────────────────────────────────┐ │   │ │
│  │  │  [Cover Art]    │  │ │ HEADER: "DAILY MIX"                │ │   │ │
│  │  │  [Collage]      │  │ │ "Based on your history" [Covers]    │ │   │ │
│  │  │                 │  │ └─────────────────────────────────────┘ │   │ │
│  │  │  [Shuffle Btn]  │  │                                         │   │ │
│  │  │                 │  │ Track 1  [Cover] Title • Artist  3:42  │   │ │
│  │  │ Click = Shuffle │  │ Track 2  [Cover] Title • Artist  4:15  │   │ │
│  │  └──────────────────┘  │ Track 3  [Cover] Title • Artist  3:28  │   │ │
│  │                        │ Track 4  [Cover] Title • Artist  5:01  │   │ │
│  │                        │                                         │   │ │
│  │                        │ [See all of Daily Mix →]                │   │ │
│  │                        └─────────────────────────────────────────┘   │ │
│  │                                                                        │ │
│  │  RECENTLY PLAYED                                                       │ │
│  │  "Recently played" [→]                                                 │ │
│  │  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐                                  │ │
│  │  │(Cover)│ │(Cover)│ │(Cover)│ │(Cover)│  Row 1                        │ │
│  │  │Title  │ │Title  │ │Title  │ │Title  │                                │ │
│  │  └──────┘ └──────┘ └──────┘ └──────┘                                  │ │
│  │  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐                                  │ │
│  │  │(Cover)│ │(Cover)│ │(Cover)│ │(Cover)│  Row 2                        │ │
│  │  │Title  │ │Title  │ │Title  │ │Title  │                                │ │
│  │  └──────┘ └──────┘ └──────┘ └──────┘                                  │ │
│  └───────────────────────────────────────────────────────────────────────┘ │
│                                                                             │
│  [Other shelves: Made For You, Continue Podcasts, Top Artists, etc.]      │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Data Flow Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         BACKEND (Rust)                                  │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────┐      │
│  │ home/mixes.rs                                                 │      │
│  │                                                                │      │
│  │ ┌──────────────┐  ┌──────────────┐  ┌──────────────┐        │      │
│  │ │ recently_    │  │ daily_mix    │  │ your_mix     │        │      │
│  │ │ played       │  │              │  │              │        │      │
│  │ │ - 30 tracks  │  │ - 30 tracks  │  │ - 60 tracks  │        │      │
│  │ │ - newest     │  │ - history    │  │ - liked      │        │      │
│  │ │ - each once  │  │ - day seed   │  │ - favorites  │        │      │
│  │ └──────────────┘  └──────────────┘  └──────────────┘        │      │
│  │                                                                │      │
│  │ Returns: HomeMixes { recent, daily, your }                   │      │
│  └──────────────────────────────────────────────────────────────┘      │
│                          │                                              │
│                          ▼                                              │
│  ┌──────────────────────────────────────────────────────────────┐      │
│  │ bridges/home.rs (CXX-QT Bridge)                              │      │
│  │                                                                │      │
│  │ Exposes to QML:                                              │      │
│  │ • recentlyPlayed: QList<QVariant> (display)                   │      │
│  │ • dailyMix: QList<QVariant> (display)                        │      │
│  │ • yourMix: QList<QVariant> (display)                         │      │
│  │ • recentTracks: QObject* (playback model)                     │      │
│  │ • dailyMixTracks: QObject* (playback model)                  │      │
│  │ • yourMixTracks: QObject* (playback model)                   │      │
│  └──────────────────────────────────────────────────────────────┘      │
└─────────────────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         QML LAYER                                        │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────┐      │
│  │ HomePage.qml                                                  │      │
│  │                                                                │      │
│  │ 1. Check hasMixes: (Home.recentlyPlayed ?? []).length > 0   │      │
│  │ 2. Inject HomeMixes below Quick Picks                         │      │
│  │ 3. Handle onSeeAll signal                                     │      │
│  └──────────────────────────────────────────────────────────────┘      │
│                          │                                              │
│                          ▼                                              │
│  ┌──────────────────────────────────────────────────────────────┐      │
│  │ HomeMixes.qml                                                 │      │
│  │                                                                │      │
│  │ YOUR MIX CARD                                                │      │
│  │ ├─ Displays: Home.yourMix                                    │      │
│  │ ├─ Click: Player.shuffleFrom(Home.yourMixTracks)            │      │
│  │ └─ Covers: Extracted from yourMix                           │      │
│  │                                                                │      │
│  │ DAILY MIX CARD                                               │      │
│  │ ├─ Displays: Home.dailyMix                                   │      │
│  │ ├─ Track click: Player.playFrom(dailyMixTracks, index)      │      │
│  │ ├─ Header click: emit seeAll("daily")                        │      │
│  │ └─ Footer click: emit seeAll("daily")                        │      │
│  │                                                                │      │
│  │ RECENTLY PLAYED PILLS                                         │      │
│  │ ├─ Displays: Home.recentlyPlayed                             │      │
│  │ ├─ Pill click: Player.playFrom(recentTracks, index)         │      │
│  │ └─ Arrow click: emit seeAll("recent")                        │      │
│  └──────────────────────────────────────────────────────────────┘      │
│                          │                                              │
│                          ▼ onSeeAll                                     │
│  ┌──────────────────────────────────────────────────────────────┐      │
│  │ Router.push("mix", { mix: "daily" | "recent" | "your" })    │      │
│  └──────────────────────────────────────────────────────────────┘      │
│                          │                                              │
│                          ▼                                              │
│  ┌──────────────────────────────────────────────────────────────┐      │
│  │ MixPage.qml                                                   │      │
│  │                                                                │      │
│  │ • Load: mix === "recent" ? Home.recentTracks                 │      │
│  │          : mix === "your" ? Home.yourMixTracks               │      │
│  │          : Home.dailyMixTracks                                │      │
│  │ • Display: Full track list                                    │      │
│  │ • Play: Player.playFrom(model, index)                         │      │
│  └──────────────────────────────────────────────────────────────┘      │
└─────────────────────────────────────────────────────────────────────────┘
```

## Interaction Flow Diagrams

### Your Mix Shuffle Flow
```
User clicks Your Mix card
        │
        ▼
Player.shuffleFrom(Home.yourMixTracks)
        │
        ▼
┌─────────────────────────────┐
│ Player Bridge (player.rs)   │
│                             │
│ • Get track list from model │
│ • Shuffle in random order   │
│ • Start playback at track 0 │
│ • Context: "Your Mix"       │
└─────────────────────────────┘
        │
        ▼
Playback starts shuffled ✓
```

### Daily Mix "See All" Flow
```
User clicks "See all of Daily Mix"
        │
        ▼
HomeMixes emits seeAll("daily")
        │
        ▼
HomePage catches signal
        │
        ▼
router.push("mix", { mix: "daily" })
        │
        ▼
┌─────────────────────────────┐
│ MixPage.qml loads           │
│                             │
│ • mix === "daily"           │
│ • model = Home.dailyMixTracks
│ • title = "Daily Mix"       │
│ • Display all 30 tracks     │
└─────────────────────────────┘
        │
        ▼
User sees full track list ✓
```

### Empty History Flow
```
App starts (no history)
        │
        ▼
┌─────────────────────────────┐
│ home/mixes.rs               │
│                             │
│ recent = []                 │
│ daily = []                  │
│ your = []                   │
└─────────────────────────────┘
        │
        ▼
┌─────────────────────────────────────┐
│ bridges/home.rs                     │
│                                     │
│ recentlyPlayed = [] (empty list)    │
│ dailyMix = [] (empty list)          │
│ yourMix = [] (empty list)           │
└─────────────────────────────────────┘
        │
        ▼
┌─────────────────────────────────────┐
│ HomePage.qml                        │
│                                     │
│ hasMixes = [].length > 0 = false    │
│ _syncShelves() skips injection      │
└─────────────────────────────────────┘
        │
        ▼
HomeMixes NOT shown ✓
```

## Component Dependencies

```
Main.qml
    │
    ├─ Router { pages: { mix: MixPage.qml } }
    │
    └─ HomePage.qml
            │
            ├─ Home { bridge }
            │     ├─ recentlyPlayed
            │     ├─ dailyMix
            │     ├─ yourMix
            │     ├─ recentTracks
            │     ├─ dailyMixTracks
            │     └─ yourMixTracks
            │
            └─ HomeMixes.qml
                    │
                    ├─ Player { bridge }
                    │     ├─ shuffleFrom(model)
                    │     └─ playFrom(model, index)
                    │
                    ├─ Router
                    │     └─ push(page, props)
                    │
                    └─ Catalog { bridge }
                          └─ requestTrackMenu(track, x, y)
```

---

*Architecture diagram created: November 12, 2026*
