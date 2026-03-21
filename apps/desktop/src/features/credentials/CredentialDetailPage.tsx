import { useNavigate, useParams } from "react-router";
import { CredentialDetail } from "./ui/CredentialDetail";

export function CredentialDetailPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();

  if (!id) {
    void navigate("/credentials");
    return null;
  }

  return <CredentialDetail credentialId={id} />;
}
