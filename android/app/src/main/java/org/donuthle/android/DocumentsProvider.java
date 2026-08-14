package org.donuthle.android;

import android.database.Cursor;
import android.database.MatrixCursor;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.DocumentsProvider;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.util.Arrays;
import java.util.Comparator;
import java.util.Locale;

public final class DocumentsProvider extends android.provider.DocumentsProvider {
    private static final String ROOT_ID = "donuthle";
    private static final String[] ROOT_PROJECTION = {
            DocumentsContract.Root.COLUMN_ROOT_ID,
            DocumentsContract.Root.COLUMN_MIME_TYPES,
            DocumentsContract.Root.COLUMN_FLAGS,
            DocumentsContract.Root.COLUMN_ICON,
            DocumentsContract.Root.COLUMN_TITLE,
            DocumentsContract.Root.COLUMN_SUMMARY,
            DocumentsContract.Root.COLUMN_DOCUMENT_ID,
            DocumentsContract.Root.COLUMN_AVAILABLE_BYTES
    };
    private static final String[] DOCUMENT_PROJECTION = {
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED,
            DocumentsContract.Document.COLUMN_FLAGS,
            DocumentsContract.Document.COLUMN_SIZE
    };

    private File base() {
        StorageLayout.ensure(getContext());
        return StorageLayout.root(getContext());
    }

    @Override
    public boolean onCreate() {
        StorageLayout.ensure(getContext());
        return true;
    }

    @Override
    public Cursor queryRoots(String[] projection) {
        MatrixCursor cursor = new MatrixCursor(projection == null ? ROOT_PROJECTION : projection);
        MatrixCursor.RowBuilder row = cursor.newRow();
        row.add(DocumentsContract.Root.COLUMN_ROOT_ID, ROOT_ID);
        row.add(DocumentsContract.Root.COLUMN_TITLE, "DonutHLE");
        row.add(DocumentsContract.Root.COLUMN_SUMMARY, "APK files, game data and logs");
        row.add(DocumentsContract.Root.COLUMN_DOCUMENT_ID, ROOT_ID);
        row.add(DocumentsContract.Root.COLUMN_MIME_TYPES, "*/*");
        row.add(DocumentsContract.Root.COLUMN_FLAGS,
                DocumentsContract.Root.FLAG_SUPPORTS_CREATE |
                        DocumentsContract.Root.FLAG_SUPPORTS_IS_CHILD);
        row.add(DocumentsContract.Root.COLUMN_AVAILABLE_BYTES, base().getFreeSpace());
        return cursor;
    }

    @Override
    public Cursor queryDocument(String documentId, String[] projection) throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(projection == null ? DOCUMENT_PROJECTION : projection);
        include(cursor, documentId, null);
        return cursor;
    }

    @Override
    public Cursor queryChildDocuments(String parentDocumentId, String[] projection, String sortOrder)
            throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(projection == null ? DOCUMENT_PROJECTION : projection);
        File parent = fileFor(parentDocumentId);
        File[] children = parent.listFiles();
        if (children == null) return cursor;
        Arrays.sort(children, Comparator.comparing(File::isFile).thenComparing(File::getName, String.CASE_INSENSITIVE_ORDER));
        for (File child : children) include(cursor, idFor(child), child);
        return cursor;
    }

    @Override
    public ParcelFileDescriptor openDocument(String documentId, String mode, CancellationSignal signal)
            throws FileNotFoundException {
        File file = fileFor(documentId);
        int access = mode.contains("w") ? ParcelFileDescriptor.MODE_READ_WRITE : ParcelFileDescriptor.MODE_READ_ONLY;
        if (mode.contains("w")) access |= ParcelFileDescriptor.MODE_CREATE;
        return ParcelFileDescriptor.open(file, access);
    }

    @Override
    public String createDocument(String parentDocumentId, String mimeType, String displayName)
            throws FileNotFoundException {
        File parent = fileFor(parentDocumentId);
        if (!parent.isDirectory()) throw new FileNotFoundException("Not a directory");
        String safeName = displayName.replaceAll("[\\\\/:*?\"<>|]", "_").trim();
        if (safeName.isEmpty()) safeName = "untitled";
        File result = unique(new File(parent, safeName));
        try {
            if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mimeType)) {
                if (!result.mkdirs()) throw new IOException("mkdir failed");
            } else if (!result.createNewFile()) {
                throw new IOException("create failed");
            }
        } catch (IOException error) {
            throw new FileNotFoundException(error.getMessage());
        }
        return idFor(result);
    }

    @Override
    public void deleteDocument(String documentId) throws FileNotFoundException {
        File file = fileFor(documentId);
        if (!deleteRecursively(file)) throw new FileNotFoundException("Could not delete " + documentId);
    }

    @Override
    public String renameDocument(String documentId, String displayName) throws FileNotFoundException {
        File source = fileFor(documentId);
        File parent = source.getParentFile();
        if (parent == null) throw new FileNotFoundException("No parent directory");
        File target = unique(new File(parent, displayName.replaceAll("[\\\\/:*?\"<>|]", "_")));
        if (!source.renameTo(target)) throw new FileNotFoundException("Could not rename document");
        return idFor(target);
    }

    @Override
    public boolean isChildDocument(String parentDocumentId, String documentId) {
        try {
            File parent = fileFor(parentDocumentId).getCanonicalFile();
            File child = fileFor(documentId).getCanonicalFile();
            return child.toPath().startsWith(parent.toPath());
        } catch (IOException error) {
            return false;
        }
    }

    private void include(MatrixCursor cursor, String documentId, File knownFile) throws FileNotFoundException {
        File file = knownFile == null ? fileFor(documentId) : knownFile;
        if (!file.exists()) throw new FileNotFoundException(documentId);
        int flags = 0;
        String mime = mime(file);
        if (file.isDirectory()) {
            mime = DocumentsContract.Document.MIME_TYPE_DIR;
            flags |= DocumentsContract.Document.FLAG_DIR_SUPPORTS_CREATE |
                    DocumentsContract.Document.FLAG_SUPPORTS_DELETE |
                    DocumentsContract.Document.FLAG_SUPPORTS_RENAME;
        } else {
            flags |= DocumentsContract.Document.FLAG_SUPPORTS_DELETE |
                    DocumentsContract.Document.FLAG_SUPPORTS_WRITE;
        }
        MatrixCursor.RowBuilder row = cursor.newRow();
        row.add(DocumentsContract.Document.COLUMN_DOCUMENT_ID, idFor(file));
        row.add(DocumentsContract.Document.COLUMN_DISPLAY_NAME, file.getName().isEmpty() ? "DonutHLE" : file.getName());
        row.add(DocumentsContract.Document.COLUMN_MIME_TYPE, mime);
        row.add(DocumentsContract.Document.COLUMN_LAST_MODIFIED, file.lastModified());
        row.add(DocumentsContract.Document.COLUMN_FLAGS, flags);
        row.add(DocumentsContract.Document.COLUMN_SIZE, file.isFile() ? file.length() : null);
    }

    private File fileFor(String documentId) throws FileNotFoundException {
        File root = base();
        if (ROOT_ID.equals(documentId)) return root;
        if (!documentId.startsWith(ROOT_ID + "/")) throw new FileNotFoundException("Unknown document");
        String relative = documentId.substring(ROOT_ID.length() + 1);
        File candidate = new File(root, relative);
        try {
            File canonicalRoot = root.getCanonicalFile();
            File canonicalCandidate = candidate.getCanonicalFile();
            if (!canonicalCandidate.toPath().startsWith(canonicalRoot.toPath())) {
                throw new FileNotFoundException("Path escapes root");
            }
            if (!canonicalCandidate.exists()) throw new FileNotFoundException(documentId);
            return canonicalCandidate;
        } catch (IOException error) {
            throw new FileNotFoundException(error.getMessage());
        }
    }

    private String idFor(File file) {
        try {
            String relative = base().getCanonicalFile().toPath().relativize(file.getCanonicalFile().toPath()).toString();
            return ROOT_ID + (relative.isEmpty() ? "" : "/" + relative.replace(File.separatorChar, '/'));
        } catch (IOException error) {
            return ROOT_ID;
        }
    }

    private static File unique(File file) {
        if (!file.exists()) return file;
        String name = file.getName();
        String extension = "";
        int dot = name.lastIndexOf('.');
        if (dot > 0) {
            extension = name.substring(dot);
            name = name.substring(0, dot);
        }
        int index = 2;
        File result;
        do {
            result = new File(file.getParentFile(), name + " (" + index++ + ")" + extension);
        } while (result.exists());
        return result;
    }

    private static boolean deleteRecursively(File file) {
        if (file.isDirectory()) {
            File[] children = file.listFiles();
            if (children != null) for (File child : children) if (!deleteRecursively(child)) return false;
        }
        return file.delete();
    }

    private static String mime(File file) {
        String extension = MimeTypeMap.getFileExtensionFromUrl(file.getName()).toLowerCase(Locale.US);
        if ("apk".equals(extension)) return "application/vnd.android.package-archive";
        String type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
        return type == null ? "application/octet-stream" : type;
    }
}
