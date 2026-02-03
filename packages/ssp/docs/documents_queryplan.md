Paper	Fokus	Kernkonzept
Selinger et al. (1979)	Access Path Selection in a Relational DBMS	Cost-Based Optimization (CBO): Der Goldstandard. Einführung von Kostenmodellen (CPU/IO) für Joins.
Graefe (1993)	The Volcano Optimizer Generator	Volcano Model: Das Standard-Iteratormodell (next()), das fast jede Engine (auch moderne Rust-Implementierungen) nutzt.
Boncz et al. (2005)	MonetDB/X100: Hyper-Pipelining Query Execution	Vectorized Execution: Wie man moderne CPUs (Caches, SIMD) nutzt, um Scans extrem schnell zu machen.
Neumann (2011)	Efficiently Compiling Efficient Query Plans for Modern Hardware	JIT Compilation: Die Basis für Systeme wie DuckDB oder HyPer, um Query-Pläne direkt in Maschinencode zu kompilieren.


# 🚀 Query Optimization & Relational Algebra: The Handbook

Dieses Dokument dient als Referenz für die Implementierung von Query-Plannern und Executoren. Es verbindet die mathematische Theorie der Relationenalgebra mit der physischen Umsetzung in einer Datenbank-Engine.

---

## 1. Das Fundament: Die "Must-Read" Paper

Diese wissenschaftlichen Arbeiten haben die Art und Weise, wie Datenbanken heute funktionieren, definiert:

### 🏆 Selinger et al. (1979)
**Titel:** *Access Path Selection in a Relational Database Management System*
* **Kern:** Einführung des **Cost-Based Optimizers (CBO)**.
* **Konzept:** Anstatt Query-Pläne stur nach Regeln abzuarbeiten, berechnet das System statistische Kosten (IO/CPU) für verschiedene Pfade und wählt den billigsten.
* **Wichtigkeit:** Erklärt die mathematische Kommutativität von Joins (A ⋈ B = B ⋈ A).

### ⚡ Boncz et al. (2005)
**Titel:** *MonetDB/X100: Hyper-Pipelining Query Execution*
* **Kern:** **Vectorized Execution**.
* **Konzept:** Moderne CPUs hassen das zeilenweise Verarbeiten (Volcano-Modell). Dieses Paper zeigt, wie man Daten in "Vektoren" (Batches) verarbeitet, um CPU-Caches optimal zu nutzen.
* **Relevanz:** Essentiell, wenn du deine Engine in Rust auf maximale Performance trimmen willst.

---

## 2. Mathematische Abkürzungen (Relational Algebra Rules)

Diese Regeln erlauben es dem Optimizer, den logischen Baum umzustrukturieren, um massiv Rechenzeit zu sparen.

### A. Selection Pushdown ($\sigma$-Pushdown)
* **Regel:** $\sigma_{C}(A \bowtie B) \equiv (\sigma_{C}(A)) \bowtie B$
* **Trick:** Filter so früh wie möglich anwenden. Warum eine Million Zeilen joinen, wenn wir durch einen Filter auf 10 Zeilen reduzieren können, *bevor* der teure Join kommt?



### B. Limit Pushdown (Early Exit)
* **Konzept:** Das `LIMIT` in die Scans drücken.
* **Trick:** Wenn die Engine weiß, dass nur 10 Zeilen benötigt werden, bricht der Scan-Operator sofort ab, sobald er 10 Treffer gefunden hat, anstatt die ganze Tabelle zu lesen.

### C. Projection Pruning
* **Konzept:** Nur die Spalten laden, die wirklich gebraucht werden.
* **Mathematik:** Wenn am Ende nur `name` ausgegeben wird, müssen `email` und `id` gar nicht erst von der Platte in den RAM geladen werden.

---

## 3. Physische Operatoren & Numerische Komplexität

Wie man eine mathematische Operation ($A \bowtie B$) in tatsächlichen Code übersetzt:

| Operation | Algorithmus | Komplexität | Bedingung / Trick |
| :--- | :--- | :--- | :--- |
| **Join** | **Hash Join** | $O(N + M)$ | Nutzt eine Hashmap im RAM. Sehr schnell, braucht aber Speicher. |
| **Join** | **Index Nested Loop**| $O(N \log M)$ | Nutzt einen vorhandenen B-Tree Index. |
| **Selection**| **Index Scan** | $O(\log N)$ | Springt direkt zum Wert im Index, statt alles zu lesen. |
| **Sort** | **Top-K Heap** | $O(N \log K)$ | Bei `LIMIT K`: Nutzt einen Min-Heap, um nur die $K$ kleinsten Werte zu halten. |



---

## 4. Struktur der Response: Projection & Subqueries

In modernen Systemen definiert die Projektion die "Shape" der Daten:

* **Flache Projektion:** Standard-SQL (Tabelle).
* **Verschachtelte Projektion:** Erzeugt hierarchische Strukturen (z.B. JSON-Bäume).
    * *Achtung:* Führt ohne Optimierung zum $N+1$ Problem, wenn für jede Zeile der Haupttabelle eine Sub-Abfrage in der Projektion ausgeführt wird.

---

## 5. Empfohlene Literatur & Suche

* **Das Standardwerk:** *Database Systems: The Complete Book* (Garcia-Molina, Ullman, Widom).
* **YouTube Begriffe:**
    * "Relational Algebra Equivalences"
    * "Query Optimizer Search Space"
    * "Neso Academy DBMS Playlist" (Für die mathematischen Grundlagen)
