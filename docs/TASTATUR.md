# Tastatur in RDP-Sitzungen

## Vorgabe: Zeichen als Unicode

remotejoe schickt getippte Zeichen an RDP-Sitzungen als **Unicode-Zeichen**, nicht als Tastendrücke. Sondertasten
(Enter, Pfeile, Rücktaste, F-Tasten) und Kürzel mit Strg, Alt oder der Windows-Taste gehen als Tastendrücke.

Der Vorteil: Es kommt immer genau das Zeichen an, das du getippt hast, egal welche Tastaturbelegung die Sitzung am
Server hat. Bei Tastendrücken entscheidet dagegen die Belegung am Server, welches Zeichen daraus wird. Passen die
Belegungen nicht zusammen, entstehen falsche oder unsichtbare Zeichen – etwa durch AltGr. In einem Kennwortfeld sieht
man das nicht; wiederholte Fehlversuche können das Konto sperren.

## Grenze: Fenster „als Administrator“

Windows nimmt Unicode-Zeichen in Fenstern mit erhöhten Rechten nicht an – zum Beispiel PowerShell oder CMD „als
Administrator“ und manche Einrichtungsprogramme. Dort kommen Buchstaben, Ziffern und Satzzeichen nicht an; Pfeile,
Enter und andere Sondertasten schon.

## Administrativer Tastaturmodus

Für solche Fenster gibt es den **administrativen Tastaturmodus**: Rechtsklick auf den Tab der RDP-Sitzung →
„Administrativer Tastaturmodus“.

- Er gilt nur für diese Sitzung und nur vorübergehend. Er endet, wenn du ihn abschaltest, den Tab schließt, die
  Verbindung getrennt oder neu aufgebaut wird oder nach der eingestellten Zeit ohne Eingabe (Vorgabe 10 Minuten).
- Solange er an ist, steht ein oranges Tastatursymbol neben dem Symbol des Tabs (die ersten Sekunden blinkend).
- Jedes Zeichen geht als feste, vollständige Folge von Tastendrücken: Umschalt bzw. AltGr drücken, Taste drücken und
  loslassen, Umschalt bzw. AltGr loslassen. AltGr ist die rechte Alt-Taste.
- Umschalt, Strg, Alt und AltGr **allein** senden nichts. Kürzel (Strg+C, Umschalt+Pfeil …) gehen als ganze Folge;
  am Server bleibt keine Umschalttaste hängen. Alt+Tab wechselt dabei nur zum vorigen Fenster.
- Zeichen auf **Tottasten** (auf Deutsch ``^ ´ ` ``) und Zeichen ohne eigene Taste (z. B. `é`, Emoji) gehen weiter als
  Unicode – in Administrator-Fenstern kommen sie also nicht an.
- Text, den remotejoe selbst eingibt (Makros, Tastenfolgen nach dem Verbinden, Zugangsdaten-Platzhalter), bleibt
  Unicode.

### Belegung

Die Tastendrücke passen nur, wenn die Belegung in remotejoe der Belegung der Sitzung am Server entspricht.

- Nennt das Ziel eine Tastaturbelegung (RDP-Option „Tastaturbelegung“), gilt diese. remotejoe meldet sie beim
  Verbindungsaufbau auch an den Server.
- Sonst gilt die Belegung unter **Einstellungen → Tastatur**. Vorgabe: Deutsch.
- Ob der Server diese Belegung wirklich verwendet, kann remotejoe nicht prüfen: Benutzerprofil oder
  Gruppenrichtlinien können eine andere vorgeben.

Angeboten werden: Deutsch (Deutschland/Österreich, Schweiz), Französisch (Frankreich, Belgien, Schweiz), Englisch
(Vereinigtes Königreich, USA), Italienisch, Spanisch, Portugiesisch, Niederländisch, Schwedisch, Finnisch, Norwegisch,
Dänisch, Polnisch (Programmierer), Tschechisch. Die Tabellen stammen aus den Belegungsdateien von Windows.

### Prüfen, was ankommt

Ein Augenschein genügt nicht – unsichtbare Zeichen sieht man nicht. In der Sitzung (PowerShell, auch als
Administrator) zeigt dieser Befehl jedes empfangene Zeichen als Zahl:

```powershell
$s = Read-Host; [int[]][char[]]$s
```

Teste damit vor einem Kennwort die Zeichen, die es enthält – an einem Testkonto, nie am echten.

### Geprüft

Automatisch getestet wird für jede Belegung die Folge jedes Zeichens, dass keine Tottaste verwendet wird und dass
Umschalttasten allein nichts senden.
