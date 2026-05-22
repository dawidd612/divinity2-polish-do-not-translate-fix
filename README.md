# Divinity II PL DO NOT TRANSLATE Fix

Prosty patcher dla polskiej wersji **Divinity II: Developer's Cut**. Program naprawia błędne napisy `DO NOT TRANSLATE` przy wybranych przedmiotach zapisanych w pliku:

```text
Data\Win32\Packed\MainDataStreaming.dv2
```

## Pobieranie

Gotowy plik `.exe` jest dostępny w zakładce **Releases**. Wystarczy pobrać najnowszą wersję i uruchomić program.


## Użycie

1. Uruchom `Divinity2-PL-DO-NOT-TRANSLATE-Fix.exe`.
2. Program spróbuje sam znaleźć folder gry.
3. Jeśli gra nie zostanie wykryta, wskaż ręcznie folder `divinity2_dev_cut`.
4. Kliknij `1. Sprawdź bez zapisu`.
5. Jeśli test przejdzie poprawnie, kliknij `2. Zrób backup i patchuj`.
6. W razie problemu użyj przycisku `Przywróć najnowszy backup`.

Program przed zapisem tworzy kopię zapasową pliku gry. Logi trafiają do folderu `_dv2_localisation_patcher_logs`, a backupy do `_dv2_localisation_patcher_backups`.

## Co zostaje zmienione w pliku

Patcher zamienia docelowe wpisy `DO NOT TRANSLATE` znajdujące się przy nazwach przedmiotów:

```text
Malachite Ore -> Ruda malachitu
Gold Ore      -> Ruda zlota
Malachite Gem -> Malachit
Sapphire      -> Szafir
Spinel        -> Spinel
```

## Ostrzeżenie Windows

Windows może pokazać ostrzeżenie SmartScreen przy pierwszym uruchomieniu pobranego pliku `.exe`. Dzieje się tak dlatego, że program jest nowy, pochodzi z internetu i nie jest podpisany płatnym certyfikatem wydawcy.

To ostrzeżenie nie oznacza automatycznie, że plik zawiera wirusa. Kod programu jest dostępny w tym repozytorium, a gotowy plik w Releases jest budowany z tego kodu. Program modyfikuje wyłącznie wskazany plik `MainDataStreaming.dv2` i przed zapisem tworzy backup.

SHA-256 pliku z wydania `v1.3.0`:

```text
29a8ee8517d1d106dd444553e07d31c8edf438338e20a39774f6b506b88b9459
```