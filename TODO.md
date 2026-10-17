# TODO

Отложенное из ТЗ и плана (`local/PLAN.md`, вне репозитория). Пункты отмечаются ✅/❌ с хешем коммита.

- **Плагины вывода Winamp (`out_*.dll`)**. Встроенный WASAPI остаётся выводом по умолчанию, плагин выбирается в настройках (файл, кнопки Config и About плагина). ABI `Out_Module` Winamp: `Init`/`Quit`, `Open(rate, ch, bits, …)`, `Write`, `CanWrite`, `IsPlaying`, `Pause`, `SetVolume`, `SetPan`, `Flush`, `GetOutputTime`/`GetWrittenTime`, плюс `hMainWindow` — скрытое окно, отвечающее на `WM_WA_IPC` так, как ждёт плагин. Плагины 32-разрядные (у пользователя `out_burst.dll` — заглушка QO Labs AudioBurst, `C:\Program Files (x86)\QO Labs\AudioBurst\audioburst.dll`), поэтому нужна либо 32-разрядная сборка MetalAMP (`i686-pc-windows-msvc`, target не установлен), либо 32-разрядный процесс-посредник с PCM через общую память. Движок уже выводит звук через trait `audio::Output`, чтобы плагин стал ещё одной реализацией.
- **Rating** (кадр POPM ID3v2) — колонка в таблицах, как в Winamp.
- **Эквалайзер / DSP** (своя цепочка: эквалайзер, усиление баса, расширение стереобазы, лимитер).
- **Тема**: сейчас только тёмная; светлая — по запросу.
