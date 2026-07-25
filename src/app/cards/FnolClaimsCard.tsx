import {
  CrmContext,
  EmptyState,
  ExtensionPointApiActions,
  Link,
  Text,
} from '@hubspot/ui-extensions';
import { hubspot } from '@hubspot/ui-extensions';

interface CrmExtensionProps {
  context: CrmContext;
  actions: ExtensionPointApiActions<'crm.record.tab'>;
}

hubspot.extend<'crm.record.tab'>(({ context, actions }: CrmExtensionProps) => (
  <FnolClaimsCard context={context} actions={actions} />
));

const FnolClaimsCard = ({ context }: CrmExtensionProps) => {
  const ticketId = context.crm.objectId;
  const portalId = context.portal.id;
  const query = `hubspotPortalId=${encodeURIComponent(portalId)}&hubspotTicketId=${encodeURIComponent(ticketId)}`;
  const fnolUrl = `https://fnol.sagesure.io/?${query}`;
  const integratedUrl = `https://app.sagesure.io/fnol?${query}`;

  return (
    <EmptyState
      title="SageSure FNOL Claim Intake"
      layout="vertical"
      imageName="documents"
    >
      <Text>
        Open this HubSpot ticket in the linked FNOL workflow while preserving
        the development-portal and ticket correlation identifiers.
      </Text>
      <Link href={fnolUrl}>Open standalone FNOL workflow</Link>
      <Link href={integratedUrl}>Open FNOL in SageSure App</Link>
    </EmptyState>
  );
};
