package org.donuthle.android;

import android.database.Cursor;
import android.database.MatrixCursor;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.webkit.MimeTypeMap;

import android.content.res.AssetFileDescriptor;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;

public final class DocumentsProvider extends android.provider.DocumentsProvider {
    private static final String ROOT_ID = "donuthle";
    private static final String[] DEFAULT_ROOT_PROJECTION = {
            DocumentsContract.Root.COLUMN_ROOT_ID,
            DocumentsContract.Root.COLUMN_MIME_TYPES,
            DocumentsContract.Root.COLUMN_FLAGS,
            DocumentsContract.Root.COLUMN_ICON,
            DocumentsContract.Root.COLUMN_TITLE,
            DocumentsContract.Root.COLUMN_DOCUMENT_ID,
            DocumentsContract.Root.COLUMN_AVAILABLE_BYTES
    };
    private static final String[] DEFAULT_DOCUMENT_PROJECTION = {
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_FLAGS,
            DocumentsContract.Document.COLUMN_SIZE,
            DocumentsContract.Document.COLUMN_LAST_MODIFIED
    };

    @Override
    public boolean onCreate() {
        StorageLayout.ensure(getContext());
        return true;
    }

    @Override
    public Cursor queryRoots(String[] projection) {
        MatrixCursor result = new MatrixCursor(projection == null ? DEFAULT_ROOT_PROJECTION : projection);
        MatrixCursor.RowBuilder row = result.newRow();
        row.add(DocumentsContract.Root.COLUMN_ROOT_ID, ROOT_ID);
        row.add(DocumentsContract.Root.COLUMN_MIME_TYPES, "*/*\napplication/vnd.android.package-archive");
        row.add(DocumentsContract.Root.COLUMN_FLAGS, DocumentsContract.Root.FLAG_SUPPORTS_CREATE);
        row.add(DocumentsContract.Root.COLUMN_ICON, android.R.drawable.ic_menu_upload);
        row.add(DocumentsContract.Root.COLUMN_TITLE, "DonutHLE");
        row.add(DocumentsContract.Root.COLUMN_DOCUMENT_ID, ROOT_ID);
        row.add(DocumentsContract.Root.COLUMN_AVAILABLE_BYTES, StorageLayout.root(getContext()).getFreeSpace());
        return result;
    }

    @Override
    public Cursor queryDocument(String documentId, String[] projection) throws FileNotFoundException {
        MatrixCursor result = new MatrixCursor(projection == null ? DEFAULT_DOCUMENT_PROJECTION : projection);
        include(result, documentId, fileFor(documentId));
        return result;
    }

    @Override
    public Cursor queryChildDocuments(String parentDocumentId, String[] projection, String sortOrder) throws FileNotFoundException {
        MatrixCursor result = new MatrixCursor(projection == null ? DEFAULT_DOCUMENT_PROJECTION : projection);
        File parent = fileFor(parentDocumentId);
        File[] files = parent.listFiles();
        if (files != null) {
            for (File file : files) include(result, idFor(file), file);
        }
        return result;
    }

    @Override
    public ParcelFileDescriptor openDocument(String documentId, String mode, CancellationSignal signal) throws FileNotFoundException {
        return ParcelFileDescriptor.open(fileFor(documentId), ParcelFileDescriptor.parseMode(mode));
    }

    @Override
    public void deleteDocument(String documentId) throws FileNotFoundException {
        File file = fileFor(documentId);
        if (file.isDirectory()) deleteTree(file);
        else if (!file.delete()) throw new FileNotFoundException("Unable to delete " + file);
    }

    @Override
    public String createDocument(String parentDocumentId, String mimeType, String displayName) throws FileNotFoundException {
        File parent = fileFor(parentDocumentId);
        File file = new File(parent, displayName);
        try {
            if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mimeType)) {
                if (!file.mkdirs() && !file.isDirectory()) throw new IOException("Unable to create folder");
            } else if (!file.createNewFile()) {
                throw new IOException("File already exists");
            }
        } catch (IOException error) {
            throw new FileNotFoundException(error.getMessage());
        }
        return idFor(file);
    }

    private void include(MatrixCursor result, String documentId, File file) {
        int flags = 0;
        if (file.isDirectory()) flags |= DocumentsContract.Document.FLAG_DIR_SUPPORTS_CREATE;
        if (!ROOT_ID.equals(documentId)) flags |= DocumentsContract.Document.FLAG_SUPPORTS_DELETE;
        MatrixCursor.RowBuilder row = result.newRow();
        row.add(DocumentsContract.Document.COLUMN_DOCUMENT_ID, documentId);
        row.add(DocumentsContract.Document.COLUMN_DISPLAY_NAME, file.getName().isEmpty() ? "DonutHLE" : file.getName());
        row.add(DocumentsContract.Document.COLUMN_MIME_TYPE, file.isDirectory() ? DocumentsContract.Document.MIME_TYPE_DIR : mimeType(file));
        row.add(DocumentsContract.Document.COLUMN_FLAGS, flags);
        row.add(DocumentsContract.Document.COLUMN_SIZE, file.isFile() ? file.length() : null);
        row.add(DocumentsContract.Document.COLUMN_LAST_MODIFIED, file.lastModified());
    }

    private File fileFor(String documentId) throws FileNotFoundException {
        File root = StorageLayout.root(getContext());
        File file = ROOT_ID.equals(documentId) ? root : new File(root, documentId.replace('/', File.separatorChar));
        try {
            String rootPath = root.getCanonicalPath();
            String filePath = file.getCanonicalPath();
            if (!filePath.equals(rootPath) && !filePath.startsWith(rootPath + File.separator)) throw new IOException("Outside root");
        } catch (IOException error) {
            throw new FileNotFoundException(error.getMessage());
        }
        if (!file.exists()) throw new FileNotFoundException(file.toString());
        return file;
    }

    private String idFor(File file) {
        return file.getAbsolutePath().substring(StorageLayout.root(getContext()).getAbsolutePath().length() + 1).replace(File.separatorChar, '/');
    }

    private String mimeType(File file) {
        String extension = MimeTypeMap.getFileExtensionFromUrl(file.getName()).toLowerCase();
        if ("apk".equals(extension)) return "application/vnd.android.package-archive";
        String type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
        return type == null ? "application/octet-stream" : type;
    }

    private void deleteTree(File file) {
        File[] children = file.listFiles();
        if (children != null) for (File child : children) deleteTree(child);
        file.delete();
    }
}
