use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use catalog::SavedReading;
use fuzzy::bundle::BUNDLE;
use fuzzy::{Analyzer, Key, Reader, Speller, keys_of, needs_reading};

const PROGRESS_EVERY: usize = 64;

/// How the library's texts read, shared by every list: what one list worked out
/// is saved in the catalog for the others and for the next launch, so the
/// dictionaries open only for a text nobody has read yet.
#[derive(Clone)]
pub struct ReadingBook {
    pub(crate) dir: PathBuf,
    pub(crate) catalog: Option<catalog::Catalog>,
    /// Lists read one at a time, each after what the last one saved. All at once
    /// on launch, they would each work out the same texts.
    pub(crate) gate: Arc<Mutex<()>>,
}

impl ReadingBook {
    /// Runs `read` with what spells texts for an [`fuzzy::Index`], then saves
    /// what it worked out. Waits for any other list: call it off the UI thread.
    pub fn read<R>(&self, read: impl FnOnce(&dyn Speller) -> R) -> R {
        self.read_with(|| Reader::open(&self.dir).ok(), |saving| read(saving))
    }

    /// Works out how the saved library reads ahead of any list asking, opening
    /// the dictionaries only for a text nobody has read yet. `on_progress` hears
    /// how many of those are done out of how many, and nothing when there are
    /// none. Waits for any list reading: call it off the UI thread.
    pub fn read_saved_library(&self, on_progress: impl Fn(usize, usize)) {
        let texts = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.texts().ok())
            .unwrap_or_default();
        self.read_texts_with(|| Reader::open(&self.dir).ok(), &texts, on_progress);
    }

    fn read_texts_with<A: Analyzer>(
        &self,
        open: impl Fn() -> Option<A>,
        texts: &[String],
        on_progress: impl Fn(usize, usize),
    ) {
        self.read_with(open, |saving| {
            let unread: Vec<&String> = texts
                .iter()
                .filter(|text| needs_reading(text) && !saving.saved.contains_key(*text))
                .collect();
            for (done, text) in unread.iter().enumerate() {
                if done.is_multiple_of(PROGRESS_EVERY) {
                    on_progress(done, unread.len());
                }
                saving.keys(text);
            }
        });
    }

    fn read_with<A: Analyzer, O: Fn() -> Option<A>, R>(
        &self,
        open: O,
        read: impl FnOnce(&Saving<A, O>) -> R,
    ) -> R {
        let _turn = self.gate.lock().unwrap_or_else(PoisonError::into_inner);
        let source = format!("{}/{}", BUNDLE.name, Reader::VERSION);
        let catalog = self.catalog.as_ref();
        // What can't be read is the same as nothing saved: it gets worked out
        let saved = catalog
            .and_then(|catalog| catalog.romaji(&source).ok())
            .unwrap_or_default();
        let saving = Saving {
            saved,
            open,
            analyzer: OnceCell::new(),
            worked_out: RefCell::default(),
        };
        let result = read(&saving);
        let worked_out = saving.worked_out.into_inner();
        if let Some(catalog) = catalog.filter(|_| !worked_out.is_empty()) {
            // Failing only means working them out again next time
            catalog.put_readings(&source, &worked_out).ok();
        }
        result
    }
}

/// Answers from what is saved, opening the analyzer the first time it can't.
struct Saving<A, O> {
    /// Each text's romaji, as [`Key::encode`] made it.
    saved: HashMap<String, String>,
    open: O,
    analyzer: OnceCell<Option<A>>,
    worked_out: RefCell<Vec<SavedReading>>,
}

impl<A: Analyzer, O: Fn() -> Option<A>> Speller for Saving<A, O> {
    fn keys(&self, text: &str) -> Vec<Key> {
        let saved = self.saved.get(text).map(|saved| Key::decode(saved));
        if let Some(Ok(keys)) = saved {
            return keys;
        }
        // Nothing is saved without an analyzer: this says nothing of how it reads
        let Some(analyzer) = self.analyzer.get_or_init(&self.open) else {
            return keys_of(text, &[]);
        };
        let readings = analyzer.readings(text);
        let keys = keys_of(text, &readings);
        if let Ok(romaji) = Key::encode(&keys) {
            self.worked_out
                .borrow_mut()
                .push(SavedReading::new(text.to_owned(), readings, romaji));
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use fuzzy::Index;

    use super::*;

    struct Reads;

    impl Analyzer for Reads {
        fn readings(&self, text: &str) -> Vec<String> {
            match text {
                "白日" => vec!["ハクジツ".to_owned()],
                _ => Vec::new(),
            }
        }
    }

    fn book(catalog: Option<catalog::Catalog>) -> ReadingBook {
        ReadingBook {
            dir: PathBuf::new(),
            catalog,
            gate: Arc::default(),
        }
    }

    fn saved_count(catalog: &catalog::Catalog) -> usize {
        let source = format!("{}/{}", BUNDLE.name, Reader::VERSION);
        catalog.romaji(&source).unwrap().len()
    }

    fn finds(book: &ReadingBook, opened: &AtomicUsize) -> bool {
        let open = || {
            opened.fetch_add(1, Ordering::Relaxed);
            Some(Reads)
        };
        book.read_with(open, |speller| {
            let index = Index::new([["白日"]], Some(speller));
            index.search("hakujitu") == [0]
        })
    }

    #[test]
    fn what_one_list_worked_out_spares_the_next_from_opening_anything() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = catalog::Catalog::open(dir.path()).unwrap();
        let opened = AtomicUsize::new(0);

        assert!(finds(&book(Some(catalog.clone())), &opened));
        assert_eq!(opened.load(Ordering::Relaxed), 1);

        assert!(finds(&book(Some(catalog)), &opened));
        assert_eq!(opened.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn saved_romaji_answers_when_no_dictionary_can() {
        let dir = tempfile::tempdir().unwrap();
        let book = book(Some(catalog::Catalog::open(dir.path()).unwrap()));
        finds(&book, &AtomicUsize::new(0));

        let found = book.read_with(
            || None::<Reads>,
            |speller| Index::new([["白日"]], Some(speller)).search("hakujitu"),
        );

        assert_eq!(found, [0]);
    }

    #[test]
    fn a_new_text_is_worked_out_and_the_rest_stay_saved() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = catalog::Catalog::open(dir.path()).unwrap();
        let book = book(Some(catalog.clone()));
        book.read_texts_with(|| Some(Reads), &["読めない".to_owned()], |_, _| {});

        let opened = AtomicUsize::new(0);
        assert!(finds(&book, &opened));

        assert_eq!(opened.load(Ordering::Relaxed), 1);
        assert_eq!(saved_count(&catalog), 2);
    }

    #[test]
    fn lists_reading_at_once_work_a_text_out_once() {
        let dir = tempfile::tempdir().unwrap();
        let book = book(Some(catalog::Catalog::open(dir.path()).unwrap()));
        let opened = AtomicUsize::new(0);

        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| finds(&book, &opened));
            }
        });

        assert_eq!(opened.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_library_read_ahead_spares_the_lists_from_opening_anything() {
        let dir = tempfile::tempdir().unwrap();
        let book = book(Some(catalog::Catalog::open(dir.path()).unwrap()));
        let opened = AtomicUsize::new(0);
        let open = || {
            opened.fetch_add(1, Ordering::Relaxed);
            Some(Reads)
        };

        book.read_texts_with(open, &["白日".to_owned(), "Lemon".to_owned()], |_, _| {});
        assert_eq!(opened.load(Ordering::Relaxed), 1);

        assert!(finds(&book, &opened));
        assert_eq!(opened.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_library_with_nothing_to_read_never_opens_the_dictionaries() {
        let book = book(None);
        let opened = AtomicUsize::new(0);
        let open = || {
            opened.fetch_add(1, Ordering::Relaxed);
            Some(Reads)
        };

        book.read_texts_with(open, &["Lemon".to_owned(), "So What".to_owned()], |_, _| {});

        assert_eq!(opened.load(Ordering::Relaxed), 0);
    }

    fn told(book: &ReadingBook, texts: &[String]) -> Vec<(usize, usize)> {
        let told = RefCell::new(Vec::new());
        book.read_texts_with(
            || Some(Reads),
            texts,
            |done, total| told.borrow_mut().push((done, total)),
        );
        told.into_inner()
    }

    #[test]
    fn progress_counts_only_what_has_yet_to_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let book = book(Some(catalog::Catalog::open(dir.path()).unwrap()));
        let first: Vec<String> = (0..10).map(|ix| format!("曲{ix}")).collect();
        told(&book, &first);

        let mut texts = first;
        texts.extend((0..PROGRESS_EVERY + 1).map(|ix| format!("歌{ix}")));
        texts.push("Lemon".to_owned());

        let total = PROGRESS_EVERY + 1;
        assert_eq!(told(&book, &texts), [(0, total), (PROGRESS_EVERY, total)]);
    }

    #[test]
    fn nothing_is_told_when_there_is_nothing_to_read() {
        let dir = tempfile::tempdir().unwrap();
        let book = book(Some(catalog::Catalog::open(dir.path()).unwrap()));
        let texts = ["白日".to_owned()];
        told(&book, &texts);

        assert!(told(&book, &texts).is_empty());
    }

    #[test]
    fn without_a_catalog_it_still_reads() {
        assert!(finds(&book(None), &AtomicUsize::new(0)));
    }

    #[test]
    fn without_an_analyzer_nothing_is_saved_and_a_text_is_still_found_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = catalog::Catalog::open(dir.path()).unwrap();
        let book = book(Some(catalog.clone()));

        let found = book.read_with(
            || None::<Reads>,
            |speller| Index::new([["カナリヤ"]], Some(speller)).search("kanariya"),
        );

        assert_eq!(found, [0]);
        assert_eq!(saved_count(&catalog), 0);
    }
}
