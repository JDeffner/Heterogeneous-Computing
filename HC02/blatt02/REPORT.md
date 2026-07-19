# Ergebnisbericht: Übungsblatt 2 (SIMT, Divergenz, Speicherbandbreite)

> **Hinweis:** Alle mit `[TODO ...]` markierten Zahlen werden nach dem
> Colab-Messlauf aus den CSVs in `results/` eingetragen (das Skript
> `scripts/plot.py` druckt genau diese Zahlen als Zusammenfassung).
> Der Bericht enthält keine Zahlen, die nicht aus den committeten CSVs stammen.

## 1. Setup

- **GPU:** [TODO: Name aus Device-Dump, z. B. Tesla T4], Compute Capability
  [TODO], [TODO] SMs, max. [TODO] Threads/SM, theoretische Peak-Bandbreite
  [TODO] GB/s (aus `2 * Speichertakt * Busbreite`, zur Laufzeit abgefragt).
- **Toolkit/Treiber:** CUDA [TODO], Treiber [TODO] (Provenienzzeile der CSVs).
- **CPU-Referenz:** Colab-Host, [TODO] Threads (OpenMP).
- **Methodik:** Pro Konfiguration 3 Warmup-Läufe, dann 10 gemessene Läufe,
  berichtet wird der Median. GPU-Zeiten mit `cudaEvent`-Paaren nur um den
  Kernel (kein H2D/D2H im Messfenster). Kein Fast-Math, `-O3`, FLOP-Zählung:
  2 FLOP pro FMA. Alle Kennzahlen stammen aus `results/*.csv`.

## 2. Aufgabe 1: SIMT und Warp-Divergenz (compute-bound)

Kernel: ein Thread pro Element, ein `float` lesen, `k = 1024` FMA-Iterationen,
ein `float` schreiben. Metrik: GFLOP/s = `2 n k / t`.

### 2.1 Experiment A: Skalierung über n

![Skalierung](results/plots/task1_scaling.png)

- Die GPU erreicht ihre volle Rechenleistung erst ab n ≈ [TODO: 2^x]
  ([TODO] Threads, entspricht [TODO] Threads/SM bei [TODO] SMs). Darunter
  liegen zu wenige Warps vor, um Latenzen zu verdecken und alle SMs zu füllen;
  zusätzlich dominiert bei sehr kleinem n der Launch-Overhead.
- Peak GPU: [TODO] GFLOP/s. CPU: [TODO] GFLOP/s (1 Thread) bzw. [TODO]
  GFLOP/s (alle Kerne), beide ab dem kleinsten n nahezu flach über n: eine
  CPU braucht keine Massenparallelität, um ihre (viel niedrigere)
  Spitzenleistung zu erreichen.
- Verhältnis GPU zu CPU (alle Kerne): ca. [TODO]x.

### 2.2 Experiment B: kontrollierte Warp-Divergenz

Divergenzgrad d ∈ {1, 2, 4, 8, 16, 32}: Verzweigung auf `threadIdx.x % d`,
ein Warp führt d verschiedene Pfade aus. Jeder Pfad leistet exakt dieselbe
Arbeit (k Iterationen, 1 FMA pro Iteration), die Pfade sind aber operativ
verschieden (eigene Konstanten und Instruktionsreihenfolge, `__noinline__`),
damit der Compiler sie nicht zu einem Pfad zusammenlegt. Erwartung bei voller
Serialisierung: Slowdown ≈ d.

![Divergenz](results/plots/task1_divergence.png)

| d | GPU-Slowdown (branch) | ideal | CPU-Slowdown (branch) |
|---|---|---|---|
| 1 | 1,00 | 1 | 1,00 |
| 2 | [TODO] | 2 | [TODO] |
| 4 | [TODO] | 4 | [TODO] |
| 8 | [TODO] | 8 | [TODO] |
| 16 | [TODO] | 16 | [TODO] |
| 32 | [TODO] | 32 | [TODO] |

- GPU: Slowdown bei d = 32 ist [TODO]x, also [TODO: nahe an / etwas unter]
  dem Ideal 1/d. Der Effekt ist real und kein Compiler-Artefakt
  (Verifikation gemäß Spezifikation: mindestens ~8x gefordert).
- CPU (OpenMP, gleiche `%d`-Verzweigung): nahezu flach über d
  (Maximum [TODO]x bei d = [TODO]). Gemessen, nicht nur behauptet.
- Sekundärvariante `looplen` (datenabhängige Schleifenlänge, auf gleiche
  Gesamtarbeit normiert): Slowdown sättigt bei ca. [TODO]x. Erwartbar, denn
  die Warp-Zeit ist das Maximum der Trip-Counts, `max(k_i) ≈ 2k` unabhängig
  von d, während die Gesamtarbeit konstant bleibt.

**Warum serialisiert die GPU divergente Pfade?** Ein Warp (32 Threads) ist
die Ausführungseinheit des SIMT-Modells: vor Volta gibt es einen einzigen
Program Counter pro Warp. Bei einer Verzweigung, die Lanes unterschiedlich
nehmen, führt die Hardware die Pfade nacheinander aus und maskiert dabei
jeweils die inaktiven Lanes (Active Mask). Bei d Pfaden ist also d-mal
hintereinander je ein Bruchteil der Lanes aktiv, die Ausführungszeit steigt
um den Faktor d. Ab Volta (Independent Thread Scheduling) hat jeder Thread
einen eigenen PC, aber pro Takt kann ein Warp-Scheduler weiterhin nur eine
Instruktion für konvergente Lane-Gruppen ausgeben: divergente
Instruktionsströme werden weiterhin zeitlich serialisiert, nur die
Verschränkung ist flexibler.

**Warum ist dieselbe Verzweigung auf der CPU fast gratis?** Jeder Kern hat
seinen eigenen, unabhängigen Kontrollfluss; es gibt keine Lock-Step-Gruppe,
die auf beide Pfade warten muss. Zusätzlich ist das Muster `i % d` perfekt
periodisch und wird vom Branch Predictor praktisch fehlerfrei vorhergesagt;
spekulative Out-of-Order-Ausführung überdeckt die verbleibenden Kosten.

## 3. Aufgabe 2: Speicherzugriff, Latency Hiding, Bandbreite (memory-bound)

Streaming-Kernel `b[j] = a[j] * c`, n = [TODO: 2^26] Elemente, Metrik:
effektive Bandbreite mit 8 B/Element (4 B lesen + 4 B schreiben).

### 3.1 Experiment C: Zugriffsmuster

![Zugriffsmuster](results/plots/task2_patterns.png)

- **Coalesced** (Stride 1): [TODO] GB/s = [TODO] % der theoretischen
  Peak-Bandbreite ([TODO] GB/s). Dieser Messwert dient als praktischer Peak
  (100 %), auf den sich alle Prozentangaben beziehen.
- **Strided:** fällt mit wachsendem Stride steil ab, ab Stride ≈ [TODO]
  nur noch [TODO] % vom Peak. Grund: pro 32er-Warp werden statt einer
  zusammenhängenden 128-B-Transaktion bis zu 32 separate Speichersegmente
  angefasst, die Busausnutzung sinkt auf 4/32 der transportierten Bytes.
- **Random Gather:** [TODO] GB/s = [TODO] % vom Peak. Fußnote zur
  Byte-Konvention: angegeben ist die Nutzdaten-Bandbreite mit 8 B/Element;
  die 4 B/Element für das Lesen des Indexarrays kommen physisch noch oben
  drauf (mit 12 B/Element gerechnet wären es [TODO] GB/s).
- Validierung: Checksummen aller Varianten stimmen mit der Referenz überein
  (relativer Fehler < 1e-5, siehe stderr-Log des Laufs).

### 3.2 Experiment D: Latency Hiding über Occupancy

Grid-Stride-Kernel (identische Gesamtarbeit), Anzahl residenter Warps
variiert über (1) Blockgröße 32 bis 1024 und (2) ungenutztes dynamisches
Shared Memory pro Block als Occupancy-Drossel bei Block 256. Occupancy ist
die theoretische aus `cudaOccupancyMaxActiveBlocksPerMultiprocessor`.

![Occupancy](results/plots/task2_occupancy.png)

- Verlauf wie erwartet: Bandbreite steigt von [TODO] GB/s bei [TODO] %
  Occupancy auf [TODO] GB/s und ist ab ca. [TODO] % Occupancy flach: dann
  sind genug Warps resident, um die Speicherlatenz vollständig zu verdecken,
  und die DRAM-Bandbreite wird zum Limit.
- Anmerkung Granularität: mit Block 256 sind auf der T4 (max. 32 Warps/SM,
  max. 4 Blöcke à 8 Warps) nur die Stufen 25/50/75/100 % erreichbar; feinere
  Stufen liefert die Blockgrößen-Serie.

**Einordnung:** Die GPU verdeckt Speicherlatenz durch Überzeichnung mit
Warps: solange genügend Warps resident sind, schaltet der Scheduler bei
jedem Speicherstall auf einen rechenbereiten Warp um, daher die
Occupancy-Abhängigkeit. Die CPU verdeckt Latenz stattdessen über die
Cache-Hierarchie und Hardware-Prefetching: sequentielle Zugriffe sind
schnell, zufällige Zugriffe erzeugen Cache- und TLB-Misses. Konsequenz für
Datenlayouts auf der GPU: Structure-of-Arrays statt Array-of-Structures,
Zugriffe so anordnen, dass ein Warp zusammenhängende 32/128-B-Transaktionen
erzeugt, Indirektion/Gather vermeiden, Daten an Transaktionsgrenzen
ausrichten bzw. padden.

## 4. Limitationen

- **Theoretische statt profilierter Occupancy:** Nsight Compute benötigt auf
  Colab Counter-Rechte, die üblicherweise fehlen ([TODO: Ergebnis des
  ncu-Versuchs eintragen]); daher der API-Wert.
- **Ein einzelnes GPU-Modell** (Colab T4), keine Aussage über andere
  Architekturen; alle Größen werden aber zur Laufzeit abgefragt.
- **Keine Kontrolle über ECC und Taktboost** auf der Cloud-GPU; der Median
  über 10 Läufe dämpft Ausreißer, ersetzt aber keine feste Taktbindung.
- CPU-Sweeps mit kleinerem n als die GPU-Sweeps (Laufzeitbudget); da die
  CPU-Leistung über n flach ist, verzerrt das den Vergleich nicht.
