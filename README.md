# pulse-pyhocon

**Accélérateur Rust/PyO3 pour [pyhocon](https://github.com/chimpler/pyhocon)** — un parseur HOCON
natif, **iso-fonctionnel**, avec **fallback transparent** vers pyhocon. Projet *Pulse by Astek*.

`pyhocon` est bâti sur `pyparsing` ; son parsing est dominé par la machinerie interprétée Python
(~99 % du temps, ~1 % en regex C). `pulse-pyhocon` réimplémente le chemin chaud en Rust et délègue
à pyhocon pour le reste — on garde l'exactitude, on gagne l'ordre de grandeur.

```python
import pulse_pyhocon
config = pulse_pyhocon.parse(text)   # dict imbriqué, identique à ConfigFactory.parse_string(text)
```

## Pourquoi

- **~1000–5000× plus rapide** que `pyhocon` sur le parsing (le goulot est du Python interprété, pas
  une C-extension — cas d'école pour Rust). Mesuré en A/B drift-immune sur des configs réalistes.
- **Iso-fonctionnel** : chaque sortie est validée contre `pyhocon` par un **oracle différentiel typé**
  (résultat *et* type d'exception), sur un large corpus + fuzz adversarial plein-Unicode.
- **Toujours correct** : ce que le chemin rapide ne couvre pas est délégué **de façon transparente**
  à `pyhocon` (`NotImplementedError` interne → fallback). Le drop-in ne « casse » jamais sur du HOCON
  que pyhocon accepte.

## Couverture du chemin rapide (Rust)

Objets, tableaux, scalaires (int — y compris > 64 bits →, float, bool/null insensibles à la casse),
clés pointées, fusion profonde, commentaires `#`/`//`, **substitutions** `${path}`/`${?path}` (type
préservé, concaténation, réfs avant/arrière, sub→sub, optionnel omis, fallback env), **includes** de
fichiers, **concaténation** d'objets (merge) et de tableaux. Parité des exceptions pyhocon
(`ConfigSubstitutionException`, `ConfigWrongTypeException`, `FileNotFoundError`).

**Délégué au fallback pyhocon** (corrects, sans gain) : `+=` (implémentation pyhocon spécifique),
clés quotées à caractères spéciaux, valeurs vides, `include url(...)`/`classpath(...)`, et — point
clé pour l'iso — **tout échec de résolution de substitution** : auto-référence (`a = ${a}`),
self-concaténation (`path = ${path}":/usr/bin"`), self-append/merge, et navigation de chemin à
travers une substitution (`${x.host}` où `x = ${base}`). Le noyau natif ne tente que le chemin
heureux ; dès qu'il ne sait pas résoudre, il délègue à pyhocon (l'oracle), qui résout ces idiomes
HOCON ou lève la bonne exception. **Garantie : jamais de divergence**, même sur ces cas limites.

## Installation

```bash
pip install pulse-pyhocon      # tire aussi pyhocon (utilisé pour le fallback)
```

Des wheels précompilées sont publiées pour Linux (manylinux/musllinux), macOS et Windows. À défaut de
wheel, l'installation compile le cœur Rust (toolchain Rust requise) ; le module reste utilisable via
le fallback pur-Python si l'extension native n'est pas disponible.

## Statut

Alpha. API : `pulse_pyhocon.parse(text) -> dict`. `pulse_pyhocon.BACKEND` vaut `"rust"` ou `"python"`.
Feuille de route : résolution native (au lieu du fallback) de l'auto-référence et de la navigation à
travers une substitution ; includes url/classpath ; et, à terme, un retour `ConfigTree` complet
(getters typés, `with_fallback`, `HOCONConverter`) pour une compatibilité d'API totale.

## Licence & crédits

Apache-2.0 (alignée sur pyhocon). `pulse-pyhocon` s'appuie sur **pyhocon** (chimpler/pyhocon, Apache-2.0)
comme référence d'iso-fonctionnalité et comme fallback. Merci à ses auteurs et contributeurs.
