# Threading in Rust

## Das Problem

GUI-Anwendungen haben ein großes Problem: Wenn eine lange Operation (wie ein MFT-Scan) auf dem Haupt-Thread läuft, friert die UI ein. Der User kann nichts klicken, das Fenster reagiert nicht.

## Die Lösung: Multi-Threading

Wir verschieben langsame Operationen in einen **Background-Thread**.

```
Main Thread          Background Thread
    │                      │
    │  spawn()             │
    ├─────────────────────>│
    │                      │ MFT-Scan
    │                      │ (dauert lange)
    │  invoke_from_loop()  │
    │<─────────────────────┤
    │                      │
    ▼                      ▼
UI bleibt              Arbeit wird
responsive             erledigt
```

## Rust's Ownership-Problem

In Rust kannst du Daten nicht einfach zwischen Threads teilen:

```rust
// Das funktioniert NICHT:
let data = vec![1, 2, 3];
std::thread::spawn(|| {
    println!("{:?}", data);  // Error: data moved!
});
println!("{:?}", data);  // Error: data already moved!
```

## Lösung 1: Clone

Die einfachste Lösung - kopiere die Daten:

```rust
let data = vec![1, 2, 3];
let data_clone = data.clone();  // Kopie erstellen

std::thread::spawn(move || {
    println!("{:?}", data_clone);  // OK!
});

println!("{:?}", data);  // OK! Original noch da
```

## Lösung 2: Arc (Atomic Reference Counting)

Wenn du Daten zwischen Threads **teilen** willst, ohne zu kopieren:

```rust
use std::sync::Arc;

let data = Arc::new(vec![1, 2, 3]);
let data_clone = Arc::clone(&data);  // Nur der Pointer wird kopiert!

std::thread::spawn(move || {
    println!("{:?}", data_clone);  // Liest die gleichen Daten
});

println!("{:?}", data);  // Gleiche Daten
```

`Arc` = **A**tomically **R**eference **C**ounted
- Mehrere Threads können den gleichen Wert besitzen
- Der Wert wird gelöscht wenn der letzte Arc weg ist
- Thread-sicher (anders als `Rc`)

## Lösung 3: Arc + Mutex

Wenn du Daten zwischen Threads **ändern** willst:

```rust
use std::sync::{Arc, Mutex};

let counter = Arc::new(Mutex::new(0));
let counter_clone = Arc::clone(&counter);

std::thread::spawn(move || {
    let mut num = counter_clone.lock().unwrap();
    *num += 1;  // Sicher ändern
});

// Main thread kann auch ändern
let mut num = counter.lock().unwrap();
*num += 10;
```

`Mutex` = **Mut**ual **Ex**clusion
- Nur ein Thread kann gleichzeitig auf die Daten zugreifen
- `.lock()` wartet bis der Mutex frei ist

## In rustree

Wir verwenden `Arc<Mutex<AppState>>` für den gemeinsamen State:

```rust
struct AppState {
    tree: Option<TreeNode>,
    expanded_paths: HashSet<String>,
}

// Im Main-Thread erstellen
let app_state = Arc::new(Mutex::new(AppState { ... }));

// Clone für den Background-Thread
let state_for_thread = app_state.clone();

std::thread::spawn(move || {
    // MFT-Scan durchführen...
    let result = perform_scan();

    // Zurück zum UI-Thread senden
    slint::invoke_from_event_loop(move || {
        let mut state = state_for_thread.lock().unwrap();
        state.tree = Some(result);
    });
});
```

## invoke_from_event_loop

Slint (wie die meisten GUI-Frameworks) erlaubt UI-Updates nur vom Main-Thread. `invoke_from_event_loop` löst das:

```rust
// Im Background-Thread:
slint::invoke_from_event_loop(move || {
    // Dieser Code läuft auf dem Main-Thread!
    window.set_status_text("Fertig!");
});
```

## Zusammenfassung

| Problem | Lösung |
|---------|--------|
| Daten zwischen Threads kopieren | `Clone` |
| Daten zwischen Threads teilen (read-only) | `Arc<T>` |
| Daten zwischen Threads teilen (read-write) | `Arc<Mutex<T>>` |
| UI vom Background-Thread updaten | `invoke_from_event_loop` |

## Wichtige Regeln

1. **Rc ist nicht thread-safe** - verwende Arc für Threads
2. **RefCell ist nicht thread-safe** - verwende Mutex für Threads
3. **UI-Updates nur vom Main-Thread** - verwende invoke_from_event_loop
4. **Mutex-Lock so kurz wie möglich halten** - andere Threads warten!
