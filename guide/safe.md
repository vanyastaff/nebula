## Memory Safety & Corruption

- **Stack overflow** - переполнение стека
- **Heap overflow** - переполнение кучи
- **Integer overflow/underflow** - переполнение целых чисел
- **Null pointer dereference** - разыменование нулевого указателя
- **Uninitialized memory** - неинициализированная память
- **Out-of-bounds access** - выход за границы массива
- **Type confusion** - путаница типов
- **Memory corruption** - повреждение памяти
- **Wild pointer** - дикий указатель
- **Memory aliasing issues** - проблемы с наложением памяти
- **Stack smashing** - разрушение стека
- **Heap spraying** - распыление кучи

## Concurrency & Parallelism

- **Race condition** - состояние гонки
- **Data race** - гонка данных
- **Deadlock** - взаимная блокировка
- **Livelock** - активная блокировка
- **Starvation** - голодание потока
- **Priority inversion** - инверсия приоритетов
- **Thread contention** - конкуренция потоков
- **ABA problem** - проблема ABA в lock-free структурах
- **Thundering herd** - эффект толпы
- **Context switching overhead** - накладные расходы переключения контекста
- **False sharing** - ложное разделение кэш-линий
- **Lock convoy** - конвой блокировок
- **Spinlock issues** - проблемы со спинлоками
- **Readers-writer starvation** - голодание читателей/писателей

## Rust-Specific Issues

- **Lifetime elision mistakes** - ошибки опущенных lifetime
- **Lifetime bounds confusion** - путаница в границах lifetime
- **Borrow checker limitations** - ограничения borrow checker
- **Interior mutability leaks** - утечки внутренней изменяемости
- **RefCell panic** - паника RefCell при runtime
- **Mutex poisoning** - отравление мьютекса
- **Send/Sync trait violations** - нарушение Send/Sync
- **Phantom data misuse** - неправильное использование PhantomData
- **Trait object safety issues** - проблемы безопасности trait объектов
- **Sized trait issues** - проблемы с Sized
- **Drop order dependencies** - зависимости порядка Drop
- **Async runtime blocking** - блокировка в async runtime
- **Pin projection issues** - проблемы с Pin проекцией
- **Macro hygiene problems** - проблемы гигиены макросов
- **Orphan rule conflicts** - конфликты правила сироты
- **Type inference failures** - сбои вывода типов
- **GAT (Generic Associated Types) complexity** - сложности GAT
- **HRTB (Higher-Ranked Trait Bounds)** issues

## Security Vulnerabilities

- **XSS (Cross-Site Scripting)** - межсайтовый скриптинг
- **SQL Injection** - SQL инъекция
- **CSRF (Cross-Site Request Forgery)** - подделка запросов
- **SSRF (Server-Side Request Forgery)** - подделка серверных запросов
- **RCE (Remote Code Execution)** - удаленное выполнение кода
- **LFI (Local File Inclusion)** - включение локальных файлов
- **RFI (Remote File Inclusion)** - включение удаленных файлов
- **Path/Directory traversal** - обход путей
- **Command injection** - инъекция команд
- **XXE (XML External Entity)** - внешние XML сущности
- **LDAP injection** - LDAP инъекция
- **Template injection** - инъекция шаблонов
- **Insecure deserialization** - небезопасная десериализация
- **Timing attacks** - атаки по времени
- **Side-channel attacks** - атаки по побочным каналам
- **Replay attacks** - атаки повтора
- **Man-in-the-middle (MITM)** - атака посредника
- **Privilege escalation** - повышение привилегий
- **Authentication bypass** - обход аутентификации
- **Session hijacking** - перехват сессии
- **Clickjacking** - кликджекинг
- **DNS rebinding** - переназначение DNS
- **TOCTOU (Time-of-check to time-of-use)** - гонка проверки и использования
- **Integer truncation** - усечение целых чисел
- **Format string vulnerability** - уязвимость строк формата
- **Cryptographic issues** - криптографические проблемы

## Performance Issues

- **N+1 query problem** - проблема N+1 запросов
- **Cache stampede** - лавина кэша
- **Premature optimization** - преждевременная оптимизация
- **Algorithmic complexity issues** (O(n²), O(2^n))
- **Memory bloat** - раздувание памяти
- **Unnecessary boxing** - избыточное boxing
- **Excessive cloning** - избыточное клонирование
- **String concatenation in loops** - конкатенация строк в циклах
- **Inefficient collections** - неэффективные коллекции
- **Cache misses** - промахи кэша
- **Branch misprediction** - неправильное предсказание ветвлений
- **Pipeline stalls** - остановки конвейера
- **TLB thrashing** - чрезмерное обращение к TLB
- **Busy waiting** - активное ожидание
- **Polling overhead** - накладные расходы опроса
- **Excessive syscalls** - избыточные системные вызовы
- **Lock-free algorithm pitfalls** - подводные камни lock-free
- **Memory bandwidth saturation** - насыщение пропускной способности памяти
- **NUMA effects** - эффекты NUMA
- **Slow path execution** - выполнение медленного пути

## API Design Issues

- **Breaking changes** - ломающие изменения
- **API bloat** - раздувание API
- **Poor naming conventions** - плохие соглашения именования
- **Inconsistent interfaces** - несогласованные интерфейсы
- **Leaky abstractions** - протекающие абстракции
- **God objects** - божественные объекты
- **Feature envy** - зависть к функциям
- **Inappropriate intimacy** - неуместная близость
- **Primitive obsession** - одержимость примитивами
- **Data clumps** - сгустки данных
- **Refused bequest** - отказанное наследство
- **Divergent change** - расходящееся изменение
- **Shotgun surgery** - хирургия дробовиком
- **Parallel inheritance hierarchies** - параллельные иерархии
- **Swiss army knife** - швейцарский нож
- **Yo-yo problem** - йо-йо проблема

## Error Handling

- **Silent failures** - тихие сбои
- **Error swallowing** - проглатывание ошибок
- **Exception masking** - маскировка исключений
- **Panic in production** - паника в продакшене
- **Unwrap/expect abuse** - злоупотребление unwrap/expect
- **Error type inflation** - раздувание типов ошибок
- **Context loss** - потеря контекста ошибки
- **Recovery impossibility** - невозможность восстановления
- **Error code hell** - ад кодов ошибок

## Resource Management

- **Resource leak** - утечка ресурсов
- **File descriptor exhaustion** - исчерпание дескрипторов
- **Connection pool exhaustion** - исчерпание пула соединений
- **Thread pool saturation** - насыщение пула потоков
- **Disk space exhaustion** - исчерпание дискового пространства
- **Handle leaks** - утечки дескрипторов
- **Socket leaks** - утечки сокетов
- **Unbounded growth** - неограниченный рост
- **Retention of stale references** - сохранение устаревших ссылок

## Architectural Anti-patterns

- **Spaghetti code** - спагетти-код
- **Big ball of mud** - большой ком грязи
- **Lava flow** - поток лавы
- **Golden hammer** - золотой молоток
- **Cargo cult programming** - карго-культ программирование
- **Not invented here (NIH)** - не изобретено здесь
- **Reinventing the wheel** - изобретение велосипеда
- **Vendor lock-in** - привязка к поставщику
- **Stovepipe system** - система дымохода
- **Design by committee** - проектирование комитетом
- **Abstraction inversion** - инверсия абстракции
- **Ambiguous viewpoint** - неоднозначная точка зрения
- **Circular dependency** - циклическая зависимость
- **Dependency hell** - ад зависимостей
- **DLL hell** - ад DLL

## Testing Issues

- **Flaky tests** - нестабильные тесты
- **Test pollution** - загрязнение тестов
- **Hidden dependencies** - скрытые зависимости
- **Test interdependence** - взаимозависимость тестов
- **Hardcoded values** - жестко закодированные значения
- **Mock overuse** - злоупотребление моками
- **Insufficient coverage** - недостаточное покрытие
- **Testing implementation not behavior** - тестирование реализации

## Database Issues

- **N+1 queries**
- **Missing indexes** - отсутствующие индексы
- **Over-indexing** - избыточная индексация
- **Lock escalation** - эскалация блокировок
- **Deadlock in transactions** - deadlock в транзакциях
- **Long-running transactions** - долгие транзакции
- **Connection leaks** - утечки соединений
- **Query timeout** - таймаут запросов
- **Table scan** - полное сканирование таблицы
- **Hot spots** - горячие точки
- **Write amplification** - усиление записи

## Async/Concurrency (Rust)

- **Blocking in async** - блокировка в async
- **Async recursion** - асинхронная рекурсия
- **Runtime mixing** - смешивание runtime
- **Send bound violations** - нарушения Send границ
- **Task starvation** - голодание задач
- **Unbounded channel growth** - неограниченный рост канала
- **Select bias** - смещение select
- **Cancellation unsafety** - небезопасная отмена
- **Future leak** - утечка Future
