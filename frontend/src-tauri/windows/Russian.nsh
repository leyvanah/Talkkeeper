; Talkkeeper — русские строки установщика.
;
; Файл в UTF-8 с BOM: без него NSIS читает его в ANSI и кириллица рассыпается.
; Тот же урок, что со скриптами PowerShell.

; --- строки, которые ждёт шаблон Tauri ---
LangString addOrReinstall ${LANG_RUSSIAN} "Добавить или переустановить"
LangString alreadyInstalled ${LANG_RUSSIAN} "Уже установлено"
LangString alreadyInstalledLong ${LANG_RUSSIAN} "${PRODUCTNAME} ${VERSION} уже установлен. Выберите, что сделать, и нажмите «Далее»."
LangString appRunning ${LANG_RUSSIAN} "{{product_name}} запущен. Закройте его и попробуйте снова."
LangString appRunningOkKill ${LANG_RUSSIAN} "{{product_name}} всё ещё запущен.$\nНажмите «ОК», чтобы закрыть его и продолжить."
LangString chooseMaintenanceOption ${LANG_RUSSIAN} "Выберите, что сделать с установленной версией."
LangString choowHowToInstall ${LANG_RUSSIAN} "Выберите, как установить ${PRODUCTNAME}."
LangString createDesktop ${LANG_RUSSIAN} "Создать ярлык на рабочем столе"
LangString dontUninstall ${LANG_RUSSIAN} "Оставить установленную версию"
LangString dontUninstallDowngrade ${LANG_RUSSIAN} "Оставить установленную версию (откат без удаления отключён)"
LangString failedToKillApp ${LANG_RUSSIAN} "Не удалось закрыть {{product_name}}. Закройте вручную и повторите."
LangString installingWebview2 ${LANG_RUSSIAN} "Установка среды Microsoft Edge WebView2…"
LangString newerVersionInstalled ${LANG_RUSSIAN} "Установлена более новая версия ${PRODUCTNAME}. Ставить сборку постарше не рекомендуется. Если версия нужна именно эта — сначала удалите текущую."
LangString older ${LANG_RUSSIAN} "более старая"
LangString olderOrUnknownVersionInstalled ${LANG_RUSSIAN} "Установлена $R4 версия ${PRODUCTNAME}. Рекомендуется сначала удалить её. Выберите вариант и нажмите «Далее»."
LangString silentDowngrades ${LANG_RUSSIAN} "Откат на старую версию недоступен в тихом режиме. Запустите установщик с окном.$\n"
LangString unableToUninstall ${LANG_RUSSIAN} "Не удалось удалить."
LangString uninstallApp ${LANG_RUSSIAN} "Удалить ${PRODUCTNAME}"
LangString uninstallBeforeInstalling ${LANG_RUSSIAN} "Удалить перед установкой"
LangString unknown ${LANG_RUSSIAN} "неизвестная"
LangString webview2AbortError ${LANG_RUSSIAN} "Не удалось установить WebView2. Без него Talkkeeper не запустится — перезапустите установщик и попробуйте ещё раз."
LangString webview2DownloadError ${LANG_RUSSIAN} "Не удалось скачать WebView2 — $0"
LangString webview2DownloadSuccess ${LANG_RUSSIAN} "Загрузчик WebView2 скачан"
LangString webview2Downloading ${LANG_RUSSIAN} "Загрузка WebView2…"
LangString webview2InstallError ${LANG_RUSSIAN} "Не удалось установить WebView2 (код $1)"
LangString webview2InstallSuccess ${LANG_RUSSIAN} "WebView2 установлен"
LangString deleteAppData ${LANG_RUSSIAN} "Удалить также встречи, модели и локальные данные"

; --- страница приветствия ---
LangString tkEyebrow ${LANG_RUSSIAN} "TALKKEEPER  /  ЛОКАЛЬНЫЕ ВСТРЕЧИ"
LangString tkWelcomeTitle ${LANG_RUSSIAN} "Встречи остаются вашими."
LangString tkWelcomeText ${LANG_RUSSIAN} "Запись, расшифровка, разметка говорящих и саммари — на вашем компьютере."
LangString tkBackendsBox ${LANG_RUSSIAN} "  ОДИН УСТАНОВЩИК, ТРИ ДВИЖКА  "
LangString tkBackendsList ${LANG_RUSSIAN} "NVIDIA CUDA    |    AMD / Intel / NVIDIA Vulkan    |    запасной CPU"
LangString tkBackendsHint ${LANG_RUSSIAN} "Подходящий определится сам."
LangString tkNoAccount ${LANG_RUSSIAN} "Без аккаунта. Без подписки. Без аналитики."
LangString tkFooter ${LANG_RUSSIAN} "Talkkeeper  ·  локальные встречи"

; --- страница выбора папки ---
LangString tkDirHeader ${LANG_RUSSIAN} "Куда установить"
LangString tkDirSub ${LANG_RUSSIAN} "Выберите папку — подойдёт и та, что предложена"
LangString tkDirIntro ${LANG_RUSSIAN} "Talkkeeper установится для вашей учётной записи Windows. Папку можно изменить ниже."
LangString tkDirFolder ${LANG_RUSSIAN} "ПАПКА"
LangString tkDirBrowse ${LANG_RUSSIAN} "Обзор…"
LangString tkDirBrowseTitle ${LANG_RUSSIAN} "Выберите папку для установки Talkkeeper"
LangString tkDirSpace ${LANG_RUSSIAN} "Потребуется около 810 МБ — приложение, модели и библиотеки для видеокарты. WebView2 при первом запуске добавит немного сверху."
LangString tkDirRuntimes ${LANG_RUSSIAN} "После копирования установщик тихо доставит WebView2, Visual C++ и библиотеки CUDA. Видеокарта NVIDIA задействуется сама, если есть драйвер."

; --- страница установки ---
LangString tkInstallHeader ${LANG_RUSSIAN} "Установка Talkkeeper"
LangString tkInstallSub ${LANG_RUSSIAN} "Копируем файлы и готовим локальные движки…"
LangString tkUpdateHeader ${LANG_RUSSIAN} "Обновление Talkkeeper"
LangString tkUpdateSub ${LANG_RUSSIAN} "Обновляем файлы и локальные движки…"
LangString tkUpdaterTitle ${LANG_RUSSIAN} "Talkkeeper — обновление"
LangString tkUpdaterFooter ${LANG_RUSSIAN} "Talkkeeper  ·  обновление"
LangString tkCancelUpdate ${LANG_RUSSIAN} "Отменить обновление"
LangString tkProgressCopy ${LANG_RUSSIAN} "Копирование файлов и настройка движков"
LangString tkProgressUpdate ${LANG_RUSSIAN} "Обновление до версии ${VERSION}"
LangString tkPercentDone ${LANG_RUSSIAN} "% готово"

; --- страница завершения ---
LangString tkFinishHeader ${LANG_RUSSIAN} "Установка завершена"
LangString tkFinishSub ${LANG_RUSSIAN} "Talkkeeper готов к работе."
LangString tkAbortHeader ${LANG_RUSSIAN} "Установка отменена"
LangString tkAbortSub ${LANG_RUSSIAN} "Изменения не завершены."
LangString tkFinishTitle ${LANG_RUSSIAN} "Talkkeeper готов."
LangString tkFinishText ${LANG_RUSSIAN} "Выбранный движок распознавания и локальные библиотеки установлены. При первом запуске будет короткая настройка: имя и проверка звука."
LangString tkLaunchNow ${LANG_RUSSIAN} "Запустить Talkkeeper"
LangString tkFinishLocal ${LANG_RUSSIAN} "Все данные остаются на этом компьютере, пока вы сами не подключите облачного провайдера."
LangString tkFinishButton ${LANG_RUSSIAN} "Готово"

; --- удаление ---
LangString tkKeyRescued ${LANG_RUSSIAN} "Ваши записи зашифрованы, и ключ к ним собирались удалить вместе с данными приложения.$\r$\n$\r$\nКопия ключа сохранена здесь:$\r$\n$DOCUMENTS\Talkkeeper-key-backup\keystore.json$\r$\n$\r$\nСохраните этот файл. Без него и без пароля записи открыть будет нельзя."
LangString tkKeyWarning ${LANG_RUSSIAN} "Вы собираетесь удалить данные приложения вместе с ключом, которым зашифрованы все ваши записи.$\r$\n$\r$\nСами записи лежат отдельно и останутся на диске, но без этого ключа и пароля открыть их будет нельзя — никогда.$\r$\n$\r$\nКопия ключа будет сохранена в «Документы\Talkkeeper-key-backup».$\r$\n$\r$\nУдалить данные приложения?"
LangString tkKeyKept ${LANG_RUSSIAN} "Копию ключа сохранить не удалось, поэтому данные приложения оставлены на месте:$\r$\n$INSTDIR\data$\r$\n$\r$\nТам лежит keystore.json — без него записи не открыть. Сохраните его сами, а потом удалите папку вручную."
